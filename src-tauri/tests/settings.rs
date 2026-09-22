use std::fs;
use winoptimizer_lib::core::settings::{
    AppSettings, ExperienceLevel, SettingsStore, SETTINGS_SCHEMA_VERSION,
};
#[test]
fn defaults_are_standard_local_and_opt_in_off() {
    let settings = AppSettings::default();

    assert_eq!(settings.schema_version, SETTINGS_SCHEMA_VERSION);
    assert_eq!(settings.experience_level, ExperienceLevel::Standard);
    assert!(!settings.labs_enabled);
    assert!(!settings.cleanup_preferences.preselect_safe_candidates);
    assert!(!settings.online_preferences.allow_metered_connections);
    assert!(!settings.telemetry_preferences.enabled);
    assert!(!settings.crash_reporting_preferences.enabled);
    assert!(!settings.update_preferences.check_automatically);
    assert!(!settings.recommendation_preferences.update_guidance);
    assert!(
        !settings
            .recommendation_preferences
            .preselect_safe_recommendations
    );
    settings.validate().unwrap();
}
#[test]
fn atomic_save_round_trips_without_leaving_temporary_files() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.json");
    let store = SettingsStore::at(&path);
    let settings = AppSettings {
        experience_level: ExperienceLevel::Advanced,
        labs_enabled: true,
        ..Default::default()
    };

    store.save(&settings).unwrap();
    assert_eq!(store.load().unwrap(), settings);
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn future_schema_is_rejected_without_overwriting_it() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.json");
    let original = br#"{"schema_version":99,"experience_level":"expert"}"#;
    fs::write(&path, original).unwrap();

    let error = SettingsStore::at(&path).load().unwrap_err();

    assert!(error
        .to_string()
        .contains("unsupported settings schema version 99"));
    assert_eq!(fs::read(path).unwrap(), original);
}
