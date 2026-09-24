from typing import TypedDict, List
import os

# LiteLLM otherwise downloads its price table during DSPy's import. All model
# calls in this engine are local; use the bundled metadata instead.
os.environ["LITELLM_LOCAL_MODEL_COST_MAP"] = "True"

from langgraph.graph import StateGraph, END
import dspy
import re
from ai_provider import health_check, with_local_provider


def _clean_code_block(text: str) -> str:
    """Extract valid PlantUML from LLM output, stripping any surrounding text."""
    text = text.strip()
    # Remove markdown code fences
    text = re.sub(r'^```[\w]*\s*\n?', '', text)
    text = re.sub(r'\n?```\s*$', '', text)
    text = text.strip()
    # Extract @startuml...@enduml block if present (ignore anything before/after)
    match = re.search(r'(@startuml.*?@enduml)', text, re.DOTALL)
    if match:
        return match.group(1).strip()
    # If no @startuml found, wrap it
    if not text.startswith('@startuml'):
        text = '@startuml\n' + text + '\n@enduml'
    return text


# --- DSPy Signatures ---

LANGUAGE_INSTRUCTIONS = {
    "en": "Write all content in English.",
    "sk": "Write all content in Slovak language (slovensky), but keep all technical terms, framework names, and industry-standard terminology in English. For example: 'Systém používa microservices architektúru s API gateway pre routing requestov.'",
}


class GenerateSection(dspy.Signature):
    """Generate a detailed architecture documentation section in Markdown.
    Be specific, professional, and thorough. Use tables, bullet points, and proper headings.
    Do NOT wrap output in ```markdown``` code blocks.
    Follow the language instruction precisely."""

    system_description: str = dspy.InputField(desc="Description of the software system")
    template_name: str = dspy.InputField(desc="Architecture template (arc42, c4, togaf)")
    section_title: str = dspy.InputField(desc="Title of the section to generate")
    language_instruction: str = dspy.InputField(desc="Language instruction for the output")
    content: str = dspy.OutputField(desc="Markdown content for this documentation section")


PLANTUML_EXAMPLES = {
    "entity-relationship": """@startuml
hide methods

class ExampleEntity << Entity >> {
  id : INT
  --
  field1 : VARCHAR(100)
  field2 : VARCHAR(50)
}

' Add relationships ONLY if multiple entities are requested:
' EntityA "1" -- "0..*" EntityB : relationship
@enduml""",
    "class": """@startuml
class ExampleClass {
  - field1: Type
  - field2: Type
  + method1(): ReturnType
}
@enduml""",
    "sequence": """@startuml
actor ActorName
ActorName -> Service1: action
Service1 -> Service2: action
Service2 --> Service1: response
Service1 --> ActorName: response
@enduml""",
    "component": """@startuml
package "PackageName" {
  [Component1]
  [Component2]
}
[Component1] --> [Component2]
@enduml""",
    "c4_context": """@startuml
!include <C4/C4_Context>

Person(user, "User", "A user of the system")
System(system, "SystemName", "What this system does")
System_Ext(external, "ExternalSystemName", "An external dependency")

Rel(user, system, "Uses")
Rel(system, external, "Calls")

SHOW_LEGEND()
@enduml""",
    "c4_container": """@startuml
!include <C4/C4_Container>

Person(user, "User", "A user of the system")
System_Boundary(c1, "SystemName") {
  Container(web, "Web Application", "Technology", "Delivers the UI")
  Container(api, "API", "Technology", "Handles business logic")
  ContainerDb(db, "Database", "Technology", "Stores data")
}
System_Ext(external, "ExternalSystemName", "An external dependency")

Rel(user, web, "Uses", "HTTPS")
Rel(web, api, "Calls", "IPC")
Rel(api, db, "Reads/writes")
Rel(api, external, "Calls", "HTTPS")

LAYOUT_WITH_LEGEND()
@enduml""",
}


class GeneratePlantUML(dspy.Signature):
    """Generate valid PlantUML code for a software architecture diagram.
    Always wrap with @startuml / @enduml. Output ONLY PlantUML code, no explanation.
    Use the syntax_example as reference for correct PlantUML syntax.
    CRITICAL RULES:
    - For NEW diagrams: generate ONLY elements explicitly mentioned, nothing extra.
    - For UPDATES (when existing_diagram is provided): You MUST keep ALL existing fields, entities, and relationships EXACTLY as they are. Only apply the specific change requested. Do NOT rename, remove, or merge any existing fields. Copy the existing diagram first, then apply ONLY the requested modification.
    - For C4 diagrams (diagram_type c4_context or c4_container): you MUST start with the PlantUML stdlib include shown in syntax_example (`!include <C4/C4_Context>` or `!include <C4/C4_Container>`). NEVER use a URL include (`!includeurl ...`) or a file include of a local/remote C4-PlantUML.puml — the local renderer has no network or file access and only accepts that exact stdlib include."""

    system_description: str = dspy.InputField(desc="User instruction — what to generate or what change to apply")
    diagram_type: str = dspy.InputField(desc="Type of diagram")
    syntax_example: str = dspy.InputField(desc="PlantUML syntax reference")
    existing_diagram: str = dspy.InputField(desc="Current diagram to modify. If provided, PRESERVE every single field and entity exactly as-is, then apply ONLY the requested change. Do NOT remove or rename any existing fields.", default="")
    context: str = dspy.InputField(desc="Additional context", default="")
    plantuml_code: str = dspy.OutputField(desc="Complete PlantUML code. If updating: MUST contain ALL original fields unchanged plus the requested modifications only.")


class GenerateMermaid(dspy.Signature):
    """Generate valid Mermaid diagram code for a software architecture diagram.
    Output ONLY Mermaid code, no explanation or code fences."""

    system_description: str = dspy.InputField(desc="Description of the software system")
    diagram_type: str = dspy.InputField(desc="Type of diagram")
    context: str = dspy.InputField(desc="Additional context or section content", default="")
    mermaid_code: str = dspy.OutputField(desc="Valid Mermaid diagram code")


class ValidateDocumentation(dspy.Signature):
    """Review architecture documentation for consistency and completeness.
    Be concise — keep response under 200 words."""

    system_description: str = dspy.InputField(desc="Description of the software system")
    section_summary: str = dspy.InputField(desc="Summary of all sections with their stats")
    review: str = dspy.OutputField(desc="Brief review noting any issues or confirming adequacy")


# --- DSPy Modules ---

class SectionGenerator(dspy.Module):
    def __init__(self):
        self.generate = dspy.ChainOfThought(GenerateSection)

    def forward(self, system_description, template_name, section_title, language="en"):
        lang_instruction = LANGUAGE_INSTRUCTIONS.get(language, LANGUAGE_INSTRUCTIONS["en"])
        result = self.generate(
            system_description=system_description,
            template_name=template_name,
            section_title=section_title,
            language_instruction=lang_instruction,
        )
        return result


def _parse_plantuml_entities(code: str) -> tuple:
    """Parse PlantUML code into entities dict and relationships list."""
    entities = {}  # name -> list of field lines
    relationships = []
    current_entity = None
    current_fields = []

    for line in code.split('\n'):
        stripped = line.strip()
        # Skip directives and comments
        if stripped.startswith('@') or stripped.startswith('!') or stripped.startswith('hide') or stripped.startswith("'"):
            continue
        # Match: class Name << Entity >> {  or  ENTITY(Name) {  or  entity Name {  or  class Name {
        m = re.match(r'(?:class|entity)\s+(\w+)(?:\s+<<.*?>>)?\s*\{?', stripped)
        if not m:
            m = re.match(r'ENTITY\((\w+)\)\s*\{?', stripped)
        if m:
            if current_entity:
                entities[current_entity] = current_fields
            current_entity = m.group(1)
            current_fields = []
            continue
        if current_entity and stripped == '}':
            entities[current_entity] = current_fields
            current_entity = None
            continue
        if current_entity and stripped and stripped != '{':
            current_fields.append(stripped)
            continue
        # Relationship lines (contain -- or --> etc.)
        if '--' in stripped:
            relationships.append(stripped)

    # Handle unclosed entity
    if current_entity:
        entities[current_entity] = current_fields

    return entities, relationships


def _merge_diagrams(existing_code: str, new_code: str) -> str:
    """Merge new LLM output with existing diagram, preserving existing entity fields."""
    existing_entities, existing_rels = _parse_plantuml_entities(existing_code)
    new_entities, new_rels = _parse_plantuml_entities(new_code)

    # Detect renamed entities: if LLM created "ModifiedX" or "UpdatedX", map back to "X"
    rename_map = {}
    for new_name in list(new_entities.keys()):
        lower = new_name.lower()
        for ex_name in existing_entities:
            if (lower.startswith('modified') or lower.startswith('updated') or lower.startswith('new')):
                suffix = re.sub(r'^(modified|updated|new)', '', lower, flags=re.IGNORECASE)
                if suffix == ex_name.lower():
                    rename_map[new_name] = ex_name
                    break
            # Also match if the new entity name matches an existing one (case insensitive update)
            if lower == ex_name.lower() and new_name != ex_name:
                rename_map[new_name] = ex_name
                break

    # Apply renames to new_entities
    for old_name, real_name in rename_map.items():
        new_entities[real_name] = new_entities.pop(old_name)

    # Fix relationship references
    fixed_rels = []
    for r in new_rels:
        for old_name, real_name in rename_map.items():
            r = r.replace(old_name, real_name)
        fixed_rels.append(r)
    new_rels = fixed_rels

    # Merge: existing entities get UPDATED (replaced) if LLM provided a new version
    merged = dict(existing_entities)
    for name, fields in new_entities.items():
        if name in merged:
            # LLM is updating this entity — use LLM's version (it should have the changes)
            merged[name] = fields
        else:
            # Truly new entity
            merged[name] = fields

    # Combine relationships (deduplicate)
    all_rels = list(existing_rels)
    for r in new_rels:
        if r not in all_rels:
            all_rels.append(r)

    # Rebuild PlantUML
    lines = ['@startuml', 'hide methods', '']
    for name, fields in merged.items():
        lines.append(f'class {name} << Entity >> {{')
        for f in fields:
            lines.append(f'  {f}')
        lines.append('}')
        lines.append('')
    for r in all_rels:
        lines.append(r)
    lines.append('@enduml')
    return '\n'.join(lines)


def _fix_erd_syntax(code: str) -> str:
    """Fix common PlantUML ERD syntax issues."""
    # Convert 'entity' IE notation to 'class' notation
    code = re.sub(r'\bentity\s+(\w+)\s*\{', r'class \1 << Entity >> {', code)
    # Convert ENTITY(X) macro calls to direct class syntax
    code = re.sub(r'ENTITY\((\w+)\)\s*\{', r'class \1 << Entity >> {', code)
    # Remove !define ENTITY lines (no longer needed)
    code = re.sub(r'^!define\s+ENTITY.*\n?', '', code, flags=re.MULTILINE)
    # Remove * markers from IE notation
    code = re.sub(r'^\s*\*\s+', '  ', code, flags=re.MULTILINE)
    # NOTE: Do NOT convert VARCHAR(100) here — keep original SQL notation in stored data.
    # Bracket conversion happens in PlantUMLPreview at render time only.
    # Add hide methods if not present
    if 'hide methods' not in code:
        code = code.replace('@startuml', '@startuml\nhide methods', 1)
    # Fix invalid syntax like: A -- "1" *{ B }
    code = re.sub(r'\*\{\s*(\w+)\s*\}', r'\1', code)
    return code


class DiagramGenerator(dspy.Module):
    def __init__(self):
        self.gen_plantuml = dspy.Predict(GeneratePlantUML)
        self.gen_mermaid = dspy.Predict(GenerateMermaid)

    def forward(self, system_description, diagram_type, output_format="plantuml", context="", existing_diagram=""):
        if output_format == "plantuml":
            example = PLANTUML_EXAMPLES.get(diagram_type, PLANTUML_EXAMPLES.get("class", ""))
            is_update = bool(existing_diagram.strip())

            if is_update:
                # For updates: ask LLM only for the NEW/changed parts
                desc = (
                    f"Based on this change request, generate ONLY the new or modified entities and relationships.\n"
                    f"Change request: {system_description}"
                )
            else:
                desc = system_description

            result = self.gen_plantuml(
                system_description=desc,
                diagram_type=diagram_type,
                syntax_example=example,
                existing_diagram=existing_diagram if is_update else "",
                context=context,
            )
            raw = result.plantuml_code
            print(f"[DiagramGenerator] raw LLM output:\n{raw}\n---", flush=True)
            cleaned = _clean_code_block(raw)

            if diagram_type == "entity-relationship":
                cleaned = _fix_erd_syntax(cleaned)

            # For updates: programmatically merge to preserve existing fields
            if is_update and diagram_type == "entity-relationship":
                cleaned = _merge_diagrams(existing_diagram, cleaned)
                print(f"[DiagramGenerator] merged:\n{cleaned}\n===", flush=True)
            else:
                print(f"[DiagramGenerator] cleaned:\n{cleaned}\n===", flush=True)

            return cleaned
        else:
            result = self.gen_mermaid(
                system_description=system_description,
                diagram_type=diagram_type,
                context=context,
            )
            return _clean_code_block(result.mermaid_code)


class DocValidator(dspy.Module):
    def __init__(self):
        self.validate = dspy.Predict(ValidateDocumentation)

    def forward(self, system_description, section_summary):
        return self.validate(
            system_description=system_description,
            section_summary=section_summary,
        )


# --- Module Instances ---
section_gen = SectionGenerator()
diagram_gen = DiagramGenerator()
doc_validator = DocValidator()


# --- State ---
class AgentState(TypedDict):
    description: str
    diagram_type: str
    output_format: str
    template: str
    language: str
    sections: List[dict]
    current_section_index: int
    diagrams: List[dict]


# --- Template Definitions ---
TEMPLATE_SECTIONS = {
    "arc42": [
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
    "c4": [
        "System Context",
        "Container Diagram",
        "Component Diagram",
        "Code / Class Diagram",
    ],
    "togaf": [
        "Architecture Vision",
        "Business Architecture",
        "Information Systems Architecture",
        "Technology Architecture",
        "Migration Planning",
    ],
    # Keep in sync with src/lib/templates.ts, which adds per-section guidance.
    "solution-design": [
        "Overview",
        "Business Context and Goals",
        "Requirements",
        "Architecture",
        "Components",
        "Data Flow",
        "Integration and APIs",
        "Data Model",
        "Security",
        "Reliability",
        "Performance Efficiency",
        "Cost Optimization",
        "Operational Excellence",
        "Alternatives Considered",
        "Risks and Open Questions",
    ],
    "adr": [
        "Status and Metadata",
        "Context and Problem Statement",
        "Decision Drivers",
        "Considered Options",
        "Decision Outcome",
        "Consequences",
        "Pros and Cons of the Options",
        "Confirmation",
        "More Information",
    ],
    "well-architected": [
        "Workload Summary",
        "Reliability",
        "Security",
        "Cost Optimization",
        "Operational Excellence",
        "Performance Efficiency",
        "Findings and Recommendations",
        "Action Plan",
    ],
    "release-notes": [
        "Summary",
        "New Features",
        "Improvements",
        "Bug Fixes",
        "Breaking Changes and Upgrade Steps",
        "Known Issues",
    ],
}


# --- Node Functions ---

def generate_sections(state: AgentState) -> dict:
    """Generate documentation content for all sections using DSPy."""
    template = state["template"]
    description = state["description"]
    section_titles = TEMPLATE_SECTIONS.get(template, ["Overview"])

    language = state.get("language", "en")

    sections = []
    for title in section_titles:
        result = section_gen(
            system_description=description,
            template_name=template,
            section_title=title,
            language=language,
        )
        sections.append({
            "title": title,
            "content": result.content,
            "diagrams": [],
        })

    return {"sections": sections}


def generate_diagrams(state: AgentState) -> dict:
    """Generate diagrams for sections that benefit from visual representation."""
    sections = state["sections"]
    description = state["description"]
    output_format = state.get("output_format", "plantuml")

    diagram_sections = {
        "arc42": {
            "3. Context and Scope": "component",
            "5. Building Block View": "component",
            "6. Runtime View": "sequence",
            "7. Deployment View": "deployment",
        },
        "c4": {
            "System Context": "c4_context",
            "Container Diagram": "c4_container",
            "Component Diagram": "component",
        },
        "togaf": {
            "Business Architecture": "activity",
            "Information Systems Architecture": "component",
            "Technology Architecture": "deployment",
        },
        "solution-design": {
            "Architecture": "component",
            "Data Flow": "sequence",
        },
    }

    template = state.get("template", "arc42")
    relevant = diagram_sections.get(template, {})

    for section in sections:
        if section["title"] in relevant:
            dtype = relevant[section["title"]]
            code = diagram_gen(
                system_description=description,
                diagram_type=dtype,
                output_format=output_format,
                context=section["content"][:1000],
            )
            section["diagrams"].append({
                "content": code,
                "diagram_type": dtype,
                "format": output_format,
            })

    return {"sections": sections}


def validate_output(state: AgentState) -> dict:
    """Validate generated content for consistency and completeness."""
    sections = state["sections"]
    description = state["description"]

    section_summary = "\n".join(
        f"- {s['title']}: {len(s['content'])} chars, {len(s['diagrams'])} diagrams"
        for s in sections
    )

    doc_validator(
        system_description=description,
        section_summary=section_summary,
    )

    return {"sections": sections}


# --- Build Graph ---
def build_graph():
    graph = StateGraph(AgentState)

    graph.add_node("generate_sections", generate_sections)
    graph.add_node("generate_diagrams", generate_diagrams)
    graph.add_node("validate_output", validate_output)

    graph.set_entry_point("generate_sections")
    graph.add_edge("generate_sections", "generate_diagrams")
    graph.add_edge("generate_diagrams", "validate_output")
    graph.add_edge("validate_output", END)

    return graph.compile()

agent_graph = build_graph()


# --- Public API (called from Rust via PyO3) ---

@with_local_provider
def run_agent(description: str, diagram_type: str, output_format: str = "plantuml", language: str = "en", existing_diagram: str = ""):
    """Generate or update a diagram."""
    code = diagram_gen(
        system_description=description,
        diagram_type=diagram_type,
        output_format=output_format,
        existing_diagram=existing_diagram,
    )
    return {
        "content": code,
        "format": output_format,
    }


@with_local_provider
def run_doc_agent(description: str, template: str, output_format: str = "plantuml", language: str = "en"):
    """Generate full documentation with diagrams — called from the prompt panel."""
    initial_state = {
        "description": description,
        "diagram_type": "",
        "output_format": output_format,
        "template": template,
        "language": language,
        "sections": [],
        "current_section_index": 0,
        "diagrams": [],
    }

    result = agent_graph.invoke(initial_state)
    return {
        "template": template,
        "sections": result["sections"],
    }
