# DisRust

<img src="assets/avatar.png" alt="Логотип DisRust" width="96">

DisRust — экспериментальное настольное приложение с собственным интерфейсом на Rust и egui. Оно показывает профиль и серверы аккаунта Discord после входа через OAuth. В разделе «Демо чата» можно попробовать редактор сообщений локально.

**В этой версии подключение к Discord поддерживает профиль и список серверов.** Обычные OAuth-разрешения `identify` и `guilds` не дают доступ к реальным каналам, перепискам и звонкам. Сообщения из демо не отправляются в Discord. Официальный Discord устанавливать для DisRust не требуется. [Разрешения Discord OAuth](https://docs.discord.com/developers/topics/oauth2#shared-resources-oauth2-scopes)

## Запуск на Windows

Без настройки OAuth сразу доступно «Демо чата». Чтобы загрузить свой профиль и серверы:

1. Создай личное приложение в [Discord Developer Portal](https://discord.com/developers/applications). В настройках **OAuth2 → Redirects** добавь `http://127.0.0.1:42817/callback`.
2. Скопируй **Client ID** и **Client Secret** из его настроек OAuth2. Это секрет одного приложения разработчика. Не публикуй его и не вставляй в исходный код.
3. В PowerShell в папке с EXE запусти:

   ```powershell
   $env:DISRUST_CLIENT_ID = Read-Host 'Discord Client ID'
   $env:DISRUST_CLIENT_SECRET = Read-Host 'Discord Client Secret' -MaskInput
   try { Start-Process -FilePath .\DisRust.exe -Wait }
   finally { Remove-Item Env:DISRUST_CLIENT_SECRET -ErrorAction SilentlyContinue }
   ```

   `-MaskInput` требует PowerShell 7.2 или новее.
4. Нажми «Войти через Discord» и подтверди разрешения в системном браузере. После возврата в DisRust появятся профиль и список серверов.

DisRust получает данные один раз при входе и держит их в памяти до выхода из приложения или аккаунта. Для обновления данных войди заново. Разрешение можно отозвать в настройках Discord в разделе «Авторизованные приложения».

## Сборка

На Windows нужны [Rust MSVC](https://rustup.rs/) и Visual Studio Build Tools с компонентом «Разработка классических приложений на C++». В корне проекта:

```powershell
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
```

Готовый файл находится в `target\release\DisRust.exe`. На Windows скрипт `.\tools\package.ps1` упаковывает EXE, README и уведомления о лицензиях зависимостей.

## Как применялась rust-anything

Документированное Discord правило выбора стандартного аватара записали как эталонную функцию TypeScript. Через `rust-anything bridge`, `record`, `build` и `diff` сверили результаты эталона с Rust-функцией из `src/avatar.rs`, которую приложение использует при загрузке аватара. Discord не публикует исходники официального настольного клиента для такого переноса. [Формула Discord](https://docs.discord.com/developers/reference#image-formatting) · [rust-anything](https://github.com/erkamyaman/rust-anything)
