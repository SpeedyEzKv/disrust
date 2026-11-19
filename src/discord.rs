use std::{
    env,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::blocking::{Client, RequestBuilder};
use serde::{Deserialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use tiny_http::{Header, Method, Response, Server};
use url::Url;

use crate::avatar::default_avatar_index;

const API: &str = "https://discord.com/api/v10";
const CALLBACK: &str = "http://127.0.0.1:42817/callback";

#[derive(Deserialize)]
pub struct User {
    pub id: String,
    pub username: String,
    pub global_name: Option<String>,
    avatar: Option<String>,
    discriminator: String,
}

#[derive(Deserialize)]
pub struct Guild {
    pub id: String,
    pub name: String,
}

pub struct Account {
    pub user: User,
    pub guilds: Vec<Guild>,
    pub avatar: Option<image::RgbaImage>,
}

#[derive(Deserialize)]
struct Token {
    access_token: String,
}

pub fn connect(cancelled: &AtomicBool) -> Result<Account, String> {
    let client_id = env::var("DISRUST_CLIENT_ID")
        .ok()
        .filter(|id| id.parse::<u64>().is_ok())
        .ok_or("Задай DISRUST_CLIENT_ID в настройках запуска.")?;
    let client_secret = env::var("DISRUST_CLIENT_SECRET")
        .ok()
        .filter(|secret| !secret.trim().is_empty())
        .ok_or("Задай DISRUST_CLIENT_SECRET в настройках запуска.")?;
    let csrf = random_value()?;
    let verifier = random_value()?;
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));

    let server = Server::http("127.0.0.1:42817")
        .map_err(|_| "Порт 42817 занят. Закрой другой экземпляр DisRust и повтори вход.")?;
    let mut authorize = Url::parse("https://discord.com/oauth2/authorize").unwrap();
    authorize.query_pairs_mut().extend_pairs([
        ("client_id", client_id.as_str()),
        ("response_type", "code"),
        ("scope", "identify guilds"),
        ("redirect_uri", CALLBACK),
        ("state", &csrf),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
    ]);
    open::that_detached(authorize.as_str())
        .map_err(|_| "Не удалось открыть страницу входа Discord в браузере.")?;
    let code = wait_for_code(&server, &csrf, cancelled)?;
    drop(server);

    let client = Client::builder()
        .user_agent(concat!("DisRust/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| "Не удалось настроить соединение с Discord.")?;
    let token: Token = read_json(
        client
            .post(format!("{API}/oauth2/token"))
            .basic_auth(&client_id, Some(&client_secret))
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code.as_str()),
                ("redirect_uri", CALLBACK),
                ("code_verifier", &verifier),
            ]),
        "Вход",
    )?;

    let user: User = read_json(
        client
            .get(format!("{API}/users/@me"))
            .bearer_auth(&token.access_token),
        "Загрузка профиля",
    )?;
    let mut guilds = fetch_guilds(&client, &token.access_token, cancelled)?;
    guilds.sort_by_cached_key(|guild| guild.name.to_lowercase());
    let avatar = fetch_avatar(&client, &user);
    Ok(Account {
        user,
        guilds,
        avatar,
    })
}

fn fetch_guilds(
    client: &Client,
    token: &str,
    cancelled: &AtomicBool,
) -> Result<Vec<Guild>, String> {
    const PAGE_SIZE: usize = 200;
    let mut guilds = Vec::new();
    let mut after = 0u64;
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return Err("Вход отменён.".into());
        }
        let page: Vec<Guild> = read_json(
            client
                .get(format!("{API}/users/@me/guilds"))
                .bearer_auth(token)
                .query(&[
                    ("limit", PAGE_SIZE.to_string()),
                    ("after", after.to_string()),
                ]),
            "Загрузка серверов",
        )?;
        let complete = page.len() < PAGE_SIZE;
        let next = page
            .iter()
            .filter_map(|guild| guild.id.parse::<u64>().ok())
            .max();
        guilds.extend(page);
        if complete {
            return Ok(guilds);
        }
        after = next
            .filter(|id| *id > after)
            .ok_or("Discord не вернул следующую страницу серверов.")?;
    }
}

fn fetch_avatar(client: &Client, user: &User) -> Option<image::RgbaImage> {
    let path = match user.avatar.as_deref() {
        Some(hash)
            if user.id.parse::<u64>().is_ok()
                && hash.len() <= 64
                && !hash.is_empty()
                && hash
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_') =>
        {
            format!("avatars/{}/{hash}.png?size=64", user.id)
        }
        _ => format!(
            "embed/avatars/{}.png",
            default_avatar_index(&user.id, &user.discriminator)?
        ),
    };
    let mut response = client
        .get(format!("https://cdn.discordapp.com/{path}"))
        .send()
        .ok()?
        .error_for_status()
        .ok()?;
    let mut bytes = Vec::new();
    use std::io::Read;
    response
        .by_ref()
        .take(256 * 1024 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 256 * 1024 {
        return None;
    }

    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(256);
    limits.max_image_height = Some(256);
    limits.max_alloc = Some(4 * 1024 * 1024);
    reader.limits(limits);
    Some(reader.decode().ok()?.into_rgba8())
}

fn random_value() -> Result<String, String> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|_| "Не удалось подготовить безопасный вход.")?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn read_json<T: DeserializeOwned>(request: RequestBuilder, operation: &str) -> Result<T, String> {
    let response = request
        .send()
        .map_err(|_| format!("{operation}: Discord недоступен."))?;
    if !response.status().is_success() {
        return Err(format!(
            "{operation}: Discord вернул HTTP {}.",
            response.status().as_u16()
        ));
    }
    response
        .json()
        .map_err(|_| format!("{operation}: неожиданный ответ Discord."))
}

fn wait_for_code(server: &Server, state: &str, cancelled: &AtomicBool) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(180);
    while Instant::now() < deadline && !cancelled.load(Ordering::Relaxed) {
        let Some(request) = server
            .recv_timeout(Duration::from_millis(300))
            .map_err(|_| "Не удалось принять ответ OAuth.")?
        else {
            continue;
        };
        let callback = if *request.method() == Method::Get {
            parse_callback(request.url(), state)
        } else {
            None
        };
        let (status, body) = match &callback {
            Some(Ok(_)) => (200, "Разрешение получено. Вернись в DisRust."),
            Some(Err(_)) => (200, "Вход отменён или отклонён. Вернись в DisRust."),
            None => (400, "DisRust ожидает ответ от Discord."),
        };
        let content_type = Header::from_bytes("Content-Type", "text/plain; charset=utf-8").unwrap();
        let _ = request.respond(
            Response::from_string(body)
                .with_status_code(status)
                .with_header(content_type),
        );
        if let Some(callback) = callback {
            return callback;
        }
    }
    if cancelled.load(Ordering::Relaxed) {
        Err("Вход отменён.".into())
    } else {
        Err("Время входа истекло. Попробуй ещё раз.".into())
    }
}

fn parse_callback(path: &str, expected_state: &str) -> Option<Result<String, String>> {
    let ("/callback", query) = path.split_once('?')? else {
        return None;
    };
    let pairs = || url::form_urlencoded::parse(query.as_bytes());
    let mut states = pairs().filter(|(key, _)| key == "state");
    if states.next()?.1 != expected_state || states.next().is_some() {
        return None;
    }
    if pairs().any(|(key, _)| key == "error") {
        return Some(Err("Авторизация отменена или отклонена в Discord.".into()));
    }
    let mut codes = pairs().filter(|(key, _)| key == "code");
    let code = codes.next()?.1.into_owned();
    if code.is_empty() || codes.next().is_some() {
        return None;
    }
    Some(Ok(code))
}

#[cfg(test)]
mod tests {
    use super::parse_callback;

    #[test]
    fn callback_requires_exact_path_and_single_matching_state() {
        assert_eq!(
            parse_callback("/callback?state=good&code=a%2Bb", "good"),
            Some(Ok("a+b".into()))
        );
        for path in [
            "/favicon.ico?state=good&code=abc",
            "/other/../callback?state=good&code=abc",
            "/callback?state=other&code=abc",
            "/callback?state=good&state=other&code=abc",
            "/callback?state=good&code=abc&code=other",
            "/callback?state=good&code=",
        ] {
            assert_eq!(parse_callback(path, "good"), None, "{path}");
        }
        assert!(matches!(
            parse_callback("/callback?state=good&error=access_denied", "good"),
            Some(Err(_))
        ));
    }
}
