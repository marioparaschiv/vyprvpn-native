fn main() -> std::io::Result<()> {
    #[cfg(windows)]
    winresource::WindowsResource::new()
        .set_icon("assets/VyprVPN.ico")
        .compile()?;

    Ok(())
}
