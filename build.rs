//! Встраивает значок в `snowdrift.exe` при сборке под Windows.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    // Только когда и собираем в Windows, и цель — Windows: при проверке для Windows с
    // другой системы компилятора ресурсов нет.
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        // без значка приложение работает, поэтому сборку из-за него не роняем
        if let Err(e) = res.compile() {
            println!("cargo:warning=не удалось встроить значок в exe: {e}");
        }
    }
}
