use anyhow::Result;

pub struct EARepository;

impl EARepository {
    pub fn new() -> Result<Self> {
        // Initialize COM for EA
        Ok(Self)
    }

    /// Not implemented: a placeholder must never report success (design E0).
    pub fn import_diagram(&self, _plantuml_content: &str) -> Result<()> {
        // Planned: CoCreateInstance("EA.Repository"), OpenFile, then packages,
        // elements, connectors and a diagram through the automation interface.
        anyhow::bail!("Export to Enterprise Architect is not implemented yet. Nothing was exported.")
    }
}
