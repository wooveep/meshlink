pub trait WireGuardManager {
    fn ensure_interface(&self, name: &str) -> Result<(), String>;
}
