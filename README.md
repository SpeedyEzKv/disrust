# DisRust для Windows

<img src="assets/avatar.png" alt="Аватар DisRust" width="112">

DisRust — лаунчер на Rust для официального настольного Discord. Запусти `DisRust.exe`, и он откроет обычное окно установленного Discord. Discord использует свой текущий аккаунт, настройки и механизм обновления.

Поддерживается обычная стабильная версия Discord, установленная для текущего пользователя Windows в `%LOCALAPPDATA%\Discord`. DisRust использует штатный `Update.exe` Discord, поэтому путь к нему не зависит от версии клиента. Права администратора не требуются.

Если Discord по этому пути отсутствует, DisRust предложит открыть [официальную страницу загрузки](https://discord.com/download). Страница откроется после подтверждения.

## Сборка на Windows

Установи [Rust для Windows](https://rustup.rs/) и Visual Studio Build Tools с компонентом «Разработка классических приложений на C++». Для сборки под Windows x64 используй стандартный Rust MSVC x64.

В PowerShell из корня проекта выполни:

```powershell
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo build --release --locked
.\target\release\DisRust.exe
```

Готовый файл находится в `target\release\DisRust.exe`. Скрипт `.\tools\package.ps1` упаковывает его в ZIP вместе с README и уведомлениями о лицензиях зависимостей.
