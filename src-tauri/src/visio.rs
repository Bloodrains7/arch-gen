use anyhow::Result;
use windows::Win32::System::Com::*;

pub struct VisioApp;

impl VisioApp {
    pub fn new() -> Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)?;
        }
        Ok(Self)
    }

    /// Not implemented: a placeholder must never report success (design E0).
    pub fn draw_diagram(&self, _plantuml_content: &str) -> Result<()> {
        // Planned: CoCreateInstance("Visio.Application"), new document and page,
        // shapes and connectors from the diagram model.
        anyhow::bail!("Export to Microsoft Visio is not implemented yet. Nothing was exported.")
    }
}
