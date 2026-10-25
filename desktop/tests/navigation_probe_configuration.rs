#[path = "../navigation_probe_config.rs"]
mod configuration;
#[path = "../navigation_probe_control.rs"]
mod control;

#[test]
fn navigation_qualification_requires_exact_bundled_isolation() {
    let config: serde_json::Value =
        serde_json::from_str(include_str!("../tauri.work-navigation-probe.conf.json")).unwrap();
    configuration::validate(&config).unwrap();
    for (pointer, replacement) in [
        ("/identifier", serde_json::json!("app.quasar")),
        ("/productName", serde_json::json!("Quasar")),
        ("/app/windows/0/title", serde_json::json!("Quasar")),
        ("/build/devUrl", serde_json::json!("http://localhost:5173")),
    ] {
        let mut changed = config.clone();
        *changed.pointer_mut(pointer).unwrap() = replacement;
        assert!(configuration::validate(&changed).is_err(), "{pointer}");
    }
    let mut missing = config;
    missing["build"].as_object_mut().unwrap().remove("devUrl");
    assert!(configuration::validate(&missing).is_err());
}
