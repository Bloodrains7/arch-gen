use pyo3::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DiagramResult {
    pub content: String,
    pub diagram_type: String,
    pub format: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DocSection {
    pub title: String,
    pub content: String,
    pub diagrams: Vec<DiagramResult>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GeneratedDoc {
    pub template: String,
    pub sections: Vec<DocSection>,
}

#[tauri::command]
pub async fn generate_diagram(
    description: String,
    diagram_type: String,
    output_format: String,
    language: String,
    existing_diagram: Option<String>,
) -> Result<DiagramResult, String> {
    let desc = description.clone();
    let dtype = diagram_type.clone();
    let fmt = output_format.clone();
    let lang = language.clone();
    let existing = existing_diagram.unwrap_or_default();

    tokio::task::spawn_blocking(move || {
        crate::python_runtime::initialize()?;
        Python::with_gil(|py| {
            let agent = py
                .import("agent")
                .map_err(|e| format!("Python import error: {}", e))?;
            let result = agent
                .call_method1("run_agent", (&desc, &dtype, &fmt, &lang, &existing))
                .map_err(|e| format!("Agent error: {}", e))?;
            let content: String = result
                .get_item("content")
                .map_err(|e| format!("Result parse error: {}", e))?
                .extract()
                .map_err(|e| format!("Extract error: {}", e))?;

            Ok(DiagramResult {
                content,
                diagram_type: dtype,
                format: fmt,
            })
        })
    })
    .await
    .map_err(|e| format!("Task error: {}", e))?
}

#[tauri::command]
pub async fn generate_documentation(
    description: String,
    template: String,
    language: String,
) -> Result<GeneratedDoc, String> {
    let desc = description.clone();
    let tmpl = template.clone();
    let lang = language.clone();

    tokio::task::spawn_blocking(move || {
        crate::python_runtime::initialize()?;
        Python::with_gil(|py| {
            let agent = py
                .import("agent")
                .map_err(|e| format!("Python import error: {}", e))?;
            let result = agent
                .call_method1("run_doc_agent", (&desc, &tmpl, "plantuml", &lang))
                .map_err(|e| format!("Agent error: {}", e))?;

            let py_sections = result
                .get_item("sections")
                .map_err(|e| format!("No sections in result: {}", e))?;

            let mut sections = Vec::new();
            for i in 0..py_sections.len().map_err(|e| e.to_string())? {
                let sec = py_sections.get_item(i).map_err(|e| e.to_string())?;
                let title: String = sec
                    .get_item("title")
                    .map_err(|e| e.to_string())?
                    .extract()
                    .map_err(|e| e.to_string())?;
                let content: String = sec
                    .get_item("content")
                    .map_err(|e| e.to_string())?
                    .extract()
                    .map_err(|e| e.to_string())?;

                let py_diagrams = sec.get_item("diagrams").map_err(|e| e.to_string())?;
                let mut diagrams = Vec::new();
                for j in 0..py_diagrams.len().map_err(|e| e.to_string())? {
                    let diag = py_diagrams.get_item(j).map_err(|e| e.to_string())?;
                    let d_content: String = diag
                        .get_item("content")
                        .map_err(|e| e.to_string())?
                        .extract()
                        .map_err(|e| e.to_string())?;
                    let d_type: String = diag
                        .get_item("diagram_type")
                        .map_err(|e| e.to_string())?
                        .extract()
                        .map_err(|e| e.to_string())?;
                    let d_format: String = diag
                        .get_item("format")
                        .map_err(|e| e.to_string())?
                        .extract()
                        .map_err(|e| e.to_string())?;
                    diagrams.push(DiagramResult {
                        content: d_content,
                        diagram_type: d_type,
                        format: d_format,
                    });
                }

                sections.push(DocSection {
                    title,
                    content,
                    diagrams,
                });
            }

            Ok(GeneratedDoc {
                template: tmpl,
                sections,
            })
        })
    })
    .await
    .map_err(|e| format!("Task error: {}", e))?
}

#[tauri::command]
pub fn export_to_tool(tool: &str, content: &str) -> Result<String, String> {
    match tool {
        "visio" => {
            let visio = crate::visio::VisioApp::new().map_err(|e| e.to_string())?;
            visio.draw_diagram(content).map_err(|e| e.to_string())?;
            Ok("Exported to Visio".into())
        }
        "ea" => {
            let ea = crate::ea::EARepository::new().map_err(|e| e.to_string())?;
            ea.import_diagram(content).map_err(|e| e.to_string())?;
            Ok("Exported to Enterprise Architect".into())
        }
        _ => Err(format!("Unknown tool: {}", tool)),
    }
}

#[tauri::command]
pub fn get_template(name: &str) -> Result<Vec<String>, String> {
    let sections = match name {
        "arc42" => vec![
            "1. Introduction and Goals",
            "2. Constraints",
            "3. Context and Scope",
            "4. Solution Strategy",
            "5. Building Block View",
            "6. Runtime View",
            "7. Deployment View",
            "8. Crosscutting Concepts",
            "9. Architecture Decisions",
            "10. Quality Requirements",
            "11. Risks and Technical Debt",
            "12. Glossary",
        ],
        "c4" => vec![
            "System Context",
            "Container Diagram",
            "Component Diagram",
            "Code / Class Diagram",
        ],
        "togaf" => vec![
            "Architecture Vision",
            "Business Architecture",
            "Information Systems Architecture",
            "Technology Architecture",
            "Migration Planning",
        ],
        _ => vec!["Overview"],
    };
    Ok(sections.into_iter().map(String::from).collect())
}
