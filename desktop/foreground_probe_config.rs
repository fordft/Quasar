//! Shared build/test identity contract for the non-shipping native witness.

pub fn validate(config: &serde_json::Value) -> std::io::Result<()> {
    use serde_json::Value;
    if config.get("identifier").and_then(Value::as_str) != Some("app.quasar.work-rendering-probe")
        || config.get("productName").and_then(Value::as_str) != Some("Quasar Work Rendering Probe")
        || config
            .pointer("/app/windows/0/title")
            .and_then(Value::as_str)
            != Some("Quasar Work Rendering Probe")
        || config.pointer("/build/devUrl") != Some(&Value::Null)
    {
        return Err(std::io::Error::other(
            "the rendering probe requires its exact isolated identity and bundled frontend",
        ));
    }
    Ok(())
}

/// Refuse restoring any prior profile/session state in a native diagnostic.
pub fn require_fresh_data_root(root: &std::path::Path) -> std::io::Result<()> {
    match std::fs::read_dir(root) {
        Ok(mut entries) => {
            if entries.next().is_none() {
                Ok(())
            } else {
                Err(std::io::Error::other(
                    "the rendering probe data root must be fresh and empty",
                ))
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err(std::io::Error::other(
            "the rendering probe data root must be fresh and empty",
        )),
    }
}
