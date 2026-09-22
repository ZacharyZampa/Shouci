fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dictionary = vocab_capture::dictionary_path(None);
    let user_db = match vocab_db::app_data_dir() {
        Ok(dir) => dir.join("user.db"),
        Err(_) => std::path::PathBuf::from("user.db"),
    };
    shouci_tui::run(&dictionary, &user_db)?;
    Ok(())
}
