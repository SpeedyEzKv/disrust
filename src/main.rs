#![windows_subsystem = "windows"]

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    use std::{
        env, fs, io,
        path::PathBuf,
        process::{Command, ExitCode},
    };

    use windows::{
        Win32::UI::{
            Shell::ShellExecuteW,
            WindowsAndMessaging::{
                IDYES, MB_DEFBUTTON2, MB_ICONERROR, MB_ICONQUESTION, MB_OK, MB_YESNO, MessageBoxW,
                SW_SHOWNORMAL,
            },
        },
        core::{PCWSTR, w},
    };

    let result = (|| -> io::Result<bool> {
        let local_app_data = env::var_os("LOCALAPPDATA")
            .filter(|path| !path.is_empty())
            .ok_or_else(|| io::Error::other("Переменная LOCALAPPDATA недоступна."))?;
        let discord_dir = PathBuf::from(local_app_data).join("Discord");
        let updater = discord_dir.join("Update.exe");
        match fs::metadata(&updater) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => return Err(io::Error::other("Путь к Update.exe не указывает на файл.")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        }

        let status = Command::new(&updater)
            .args(["--processStart", "Discord.exe"])
            .current_dir(&discord_dir)
            .status()?;
        if !status.success() {
            return Err(io::Error::other(format!("Discord Update.exe: {status}")));
        }
        Ok(true)
    })();

    let message = match result {
        Ok(true) => return ExitCode::SUCCESS,
        Ok(false) => {
            // The dialog and URL are static, null-terminated strings.
            let answer = unsafe {
                MessageBoxW(
                    None,
                    w!(
                        "Discord не найден. Открыть официальную страницу загрузки?\n\nhttps://discord.com/download"
                    ),
                    w!("DisRust"),
                    MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
                )
            };
            if answer != IDYES {
                return ExitCode::FAILURE;
            }
            let opened = unsafe {
                ShellExecuteW(
                    None,
                    w!("open"),
                    w!("https://discord.com/download"),
                    None,
                    None,
                    SW_SHOWNORMAL,
                )
            };
            if opened.0 as isize > 32 {
                return ExitCode::FAILURE;
            }
            "Не удалось открыть страницу загрузки.\n\nhttps://discord.com/download".to_owned()
        }
        Err(error) => format!("Не удалось запустить Discord.\n\n{error}"),
    };

    let message: Vec<u16> = message.encode_utf16().chain([0]).collect();
    // The UTF-16 buffer remains valid until the dialog closes.
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(message.as_ptr()),
            w!("DisRust"),
            MB_OK | MB_ICONERROR,
        )
    };
    ExitCode::FAILURE
}

#[cfg(not(windows))]
fn main() -> std::process::ExitCode {
    eprintln!("DisRust запускает настольный Discord на Windows.");
    std::process::ExitCode::FAILURE
}
