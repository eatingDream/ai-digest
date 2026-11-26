pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn version_string() -> String {
    format!("ai-digest {VERSION}")
}
