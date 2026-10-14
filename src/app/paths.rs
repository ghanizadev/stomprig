use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn application_data_directory(base: &Path) -> PathBuf {
    migrate_legacy_directory(&base.join("rack"), &base.join("stomprig"))
}

pub(crate) fn migrate_legacy_directory(legacy_path: &Path, current_path: &Path) -> PathBuf {
    if current_path.exists() || !legacy_path.exists() {
        return current_path.to_path_buf();
    }

    match fs::rename(legacy_path, current_path) {
        Ok(()) => current_path.to_path_buf(),
        Err(_) if current_path.exists() => current_path.to_path_buf(),
        Err(_) => legacy_path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::application_data_directory;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn migrates_existing_application_data_to_the_new_directory() {
        let unique_id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "stomprig-path-migration-{}-{unique_id}",
            std::process::id()
        ));
        let legacy_directory = base.join("rack");
        fs::create_dir_all(&legacy_directory).expect("legacy directory should be created");
        fs::write(legacy_directory.join("settings.toml"), b"settings")
            .expect("legacy settings should be written");

        let current_directory = application_data_directory(&base);

        assert_eq!(current_directory, base.join("stomprig"));
        assert_eq!(
            fs::read(current_directory.join("settings.toml")).expect("settings should migrate"),
            b"settings"
        );
        fs::remove_dir_all(base).expect("migration directory should be removed");
    }
}
