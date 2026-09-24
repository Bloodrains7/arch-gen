// Document templates: the section structure a new document starts from, with a
// short guidance comment per section. The guidance is an HTML comment, so it is
// visible while writing but never shows in the preview or in exports.
//
// Section titles of arc42, C4, TOGAF, ADR, solution design, Well-Architected review
// and release notes must stay in sync with TEMPLATE_SECTIONS in python-engine/agent.py,
// which generates the same sections with AI.
import type { Section } from "./project";

export interface TemplateSection { title: string; guidance: string }

export interface DocumentTemplate {
  id: string;
  name: string;
  desc: string;
  group: "Architecture" | "Design & decisions" | "Delivery";
  sections: TemplateSection[];
}

const s = (title: string, guidance: string): TemplateSection => ({ title, guidance });

export const DOCUMENT_TEMPLATES: DocumentTemplate[] = [
  {
    id: "arc42", name: "arc42", desc: "Pragmatic architecture documentation", group: "Architecture",
    sections: [
      s("1. Introduction and Goals", "Key functional requirements, the top 3–5 quality goals and the stakeholders with their expectations."),
      s("2. Constraints", "Technical, organisational and legal constraints the solution must respect, and conventions."),
      s("3. Context and Scope", "The system as a black box: users and neighbouring systems, business and technical interfaces. Add a context diagram."),
      s("4. Solution Strategy", "The fundamental decisions: technology, top-level decomposition, how the quality goals are achieved."),
      s("5. Building Block View", "Static decomposition into building blocks (level 1, then refine), with responsibilities and interfaces."),
      s("6. Runtime View", "Important scenarios as sequences: how building blocks cooperate at runtime, including error paths."),
      s("7. Deployment View", "Infrastructure, environments and the mapping of building blocks to nodes."),
      s("8. Crosscutting Concepts", "Concepts that apply in many places: domain model, security, persistence, logging, error handling, i18n."),
      s("9. Architecture Decisions", "Important, expensive or risky decisions with their reasons. Link the ADR documents."),
      s("10. Quality Requirements", "Quality tree and concrete, measurable quality scenarios."),
      s("11. Risks and Technical Debt", "Known risks and debt, ordered by priority, with planned mitigation."),
      s("12. Glossary", "Domain and technical terms the stakeholders must understand in the same way."),
    ],
  },
  {
    id: "c4", name: "C4 Model", desc: "Context, Container, Component, Code", group: "Architecture",
    sections: [
      s("System Context", "The system, its users and the external systems it depends on. One C4 context diagram."),
      s("Container Diagram", "Deployable/runnable units (apps, services, databases), their technology and how they communicate."),
      s("Component Diagram", "Components inside one container, their responsibilities and dependencies."),
      s("Code / Class Diagram", "Only where it helps: the key classes or tables of one component."),
    ],
  },
  {
    id: "togaf", name: "TOGAF", desc: "Enterprise architecture framework", group: "Architecture",
    sections: [
      s("Architecture Vision", "Scope, stakeholders, business goals and drivers, and the target in one page."),
      s("Business Architecture", "Capabilities, processes and organisation affected; baseline and target."),
      s("Information Systems Architecture", "Applications and data: baseline, target and the gap between them."),
      s("Technology Architecture", "Platforms, infrastructure and standards the target needs."),
      s("Migration Planning", "Work packages, transition architectures, dependencies and risks."),
    ],
  },
  {
    id: "solution-design", name: "Solution Design", desc: "Design document for one solution or change", group: "Design & decisions",
    sections: [
      s("Overview", "What is being built and why, in a few sentences. Link the business request and related documents."),
      s("Business Context and Goals", "The problem, the users, measurable goals and what is out of scope."),
      s("Requirements", "Functional requirements and non-functional requirements (availability, latency, volumes, RPO/RTO, compliance)."),
      s("Architecture", "The target architecture with a diagram: main components, boundaries and external systems."),
      s("Components", "Each component's responsibility, technology and owner."),
      s("Data Flow", "How data moves through the solution for the main scenarios. A sequence diagram helps."),
      s("Integration and APIs", "Interfaces offered and consumed: protocol, contract, authentication, error handling, versioning."),
      s("Data Model", "Entities, ownership, retention and classification of the data."),
      s("Security", "Identity and access, secrets, encryption in transit and at rest, threat model highlights."),
      s("Reliability", "Failure modes, redundancy, retries/timeouts, backup and recovery targets."),
      s("Performance Efficiency", "Expected load, scaling approach and performance budgets."),
      s("Cost Optimization", "Main cost drivers, estimate and how cost is kept under control."),
      s("Operational Excellence", "Deployment pipeline, monitoring, alerting, runbooks and ownership."),
      s("Alternatives Considered", "Options that were evaluated and why they were not chosen."),
      s("Risks and Open Questions", "Open decisions, assumptions to validate and risks with owners."),
    ],
  },
  {
    id: "adr", name: "Decision Record (ADR)", desc: "One architecture decision, MADR style", group: "Design & decisions",
    sections: [
      s("Status and Metadata", "Status: proposed | accepted | rejected | deprecated | superseded by ADR-NNNN. Date, deciders, consulted, informed."),
      s("Context and Problem Statement", "The situation and the question to decide, in two or three sentences."),
      s("Decision Drivers", "Forces and quality goals that matter for this decision."),
      s("Considered Options", "The options, one line each."),
      s("Decision Outcome", "Chosen option: …, because … (tie it to the decision drivers)."),
      s("Consequences", "Good, bad and neutral consequences, including follow-up work."),
      s("Pros and Cons of the Options", "For each option: good, because …; bad, because …"),
      s("Confirmation", "How compliance with the decision is checked (review, test, fitness function)."),
      s("More Information", "Links to evidence, related decisions and when to revisit this decision."),
    ],
  },
  {
    id: "well-architected", name: "Well-Architected Review", desc: "Review of a workload against the five pillars", group: "Design & decisions",
    sections: [
      s("Workload Summary", "Workload, business criticality, users, environments and the scope of this review."),
      s("Reliability", "Resiliency, availability targets, failure mode analysis, recovery, health modelling."),
      s("Security", "Identity, network, data protection, secrets, threat detection and response."),
      s("Cost Optimization", "Cost model, rightsizing, budgets and cost ownership."),
      s("Operational Excellence", "DevOps practices, automation, observability, safe deployment, incident management."),
      s("Performance Efficiency", "Capacity planning, scaling, performance testing and bottlenecks."),
      s("Findings and Recommendations", "Findings per pillar with severity, evidence and recommendation."),
      s("Action Plan", "Prioritised actions with owner and target date."),
    ],
  },
  {
    id: "release-notes", name: "Release Notes", desc: "What changed in a release; fill it from Git", group: "Delivery",
    sections: [
      s("Summary", "What the release brings to users, in two or three sentences. Use Release notes from Git to fill the change lists."),
      s("New Features", "New capabilities, written for the reader of these notes."),
      s("Improvements", "Changes to existing behaviour, performance and usability."),
      s("Bug Fixes", "Fixed problems as users experienced them."),
      s("Breaking Changes and Upgrade Steps", "What stops working and exactly what to do when upgrading."),
      s("Known Issues", "Known problems and their workarounds."),
    ],
  },
];

export const CUSTOM_TEMPLATE: DocumentTemplate = {
  id: "custom", name: "Custom", desc: "Your own template structure", group: "Design & decisions",
  sections: [{ title: "Overview", guidance: "" }],
};

export function findTemplate(id: string): DocumentTemplate | undefined {
  return id === "custom" ? CUSTOM_TEMPLATE : DOCUMENT_TEMPLATES.find(t => t.id === id);
}

const guidanceComment = (guidance: string) => (guidance ? `<!-- ${guidance.replace(/--+>?/g, "–")} -->` : "");

/** A section counts as empty when it holds nothing but guidance comments. */
export function isBlankSection(section: Section): boolean {
  return section.diagrams.length === 0 && !section.content.replace(/<!--[\s\S]*?-->/g, "").trim();
}

const key = (title: string) => title.trim().toLowerCase();

/**
 * The template's structure on top of existing content, never losing any of it:
 * a section whose title matches keeps its content, identity and diagrams; other
 * sections with content stay after the template sections; only blank ones go.
 */
export function applyTemplateStructure(existing: Section[], template: DocumentTemplate): Section[] {
  // "Custom" means the structure the user already has.
  if (template.id === CUSTOM_TEMPLATE.id && existing.some(section => !isBlankSection(section))) return existing;
  const used = new Set<Section>();
  const structured = template.sections.map(({ title, guidance }) => {
    const match = existing.find(section => !used.has(section) && key(section.title) === key(title));
    if (match) { used.add(match); return match; }
    return { title, content: guidanceComment(guidance), diagrams: [] };
  });
  return [...structured, ...existing.filter(section => !used.has(section) && !isBlankSection(section))];
}
