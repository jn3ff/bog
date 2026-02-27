use std::collections::HashMap;
use std::fmt;

use crate::config::AgentRole;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Green,
    Yellow,
    Red,
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Status::Green => write!(f, "green"),
            Status::Yellow => write!(f, "yellow"),
            Status::Red => write!(f, "red"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    String(String),
    Status(Status),
    Bool(bool),
    Number(i64),
    Ident(String),
    Path(Vec<String>),
    FnRef(String),
    List(Vec<Value>),
    Tuple(Vec<Value>),
    Block(Vec<(String, Value)>),
}

#[derive(Debug, Clone)]
pub struct BogFile {
    pub annotations: Vec<Annotation>,
}

#[derive(Debug, Clone)]
pub enum Annotation {
    Repo(RepoAnnotation),
    File(FileAnnotation),
    Description(String),
    Health(HealthAnnotation),
    Fn(FnAnnotation),
    Subsystem(SubsystemDecl),
    Skimsystem(SkimsystemDecl),
    Skim(SkimObservation),
    Policies(PoliciesAnnotation),
    ChangeRequests(Vec<ChangeRequest>),
    Pickled(PickledAnnotation),
}

#[derive(Debug, Clone)]
pub struct RepoAnnotation {
    pub name: String,
    pub version: String,
    pub updated: String,
}

#[derive(Debug, Clone)]
pub struct FileAnnotation {
    pub owner: String,
    pub subsystem: String,
    pub updated: String,
    pub status: Status,
}

#[derive(Debug, Clone)]
pub struct HealthAnnotation {
    pub dimensions: HashMap<String, Status>,
}

#[derive(Debug, Clone)]
pub struct FnAnnotation {
    pub name: String,
    pub status: Status,
    pub stub: bool,
    pub deps: Vec<String>,
    pub refs: Vec<String>,
    pub contract: Option<Contract>,
    pub description: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Contract {
    pub inputs: Vec<(String, String)>,
    pub output: Option<String>,
    pub invariants: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SubsystemDecl {
    pub name: String,
    pub owner: String,
    pub files: Vec<String>,
    pub status: Status,
    pub description: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SkimsystemDecl {
    pub name: String,
    pub owner: String,
    pub targets: SkimTargets,
    pub status: Status,
    pub principles: Vec<String>,
    pub integrations: Vec<IntegrationSpec>,
    pub description: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone)]
pub struct IntegrationSpec {
    pub name: String,
    pub command: String,
    pub format: IntegrationFormat,
}

#[derive(Debug, Clone, PartialEq)]
pub enum IntegrationFormat {
    CargoDiagnostic,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SkimTargets {
    All,
    Named(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct SkimObservation {
    pub skimsystem: String,
    pub status: Status,
    pub notes: Option<String>,
    pub target: Option<SkimTarget>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SkimTarget {
    File,
    Fn(String),
}

#[derive(Debug, Clone)]
pub struct PoliciesAnnotation {
    pub fields: HashMap<String, Value>,
}

#[derive(Debug, Clone)]
pub struct PickledAnnotation {
    pub id: String,
    pub agent: String,
    pub updated: String,
    pub kind: PickledKind,
    pub supersedes: Option<String>,
    pub tags: Vec<PickledTag>,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickledKind {
    /// A deliberate choice with rationale — the ADR core
    Decision,
    /// Overturning or amending a previous decision
    Reversal,
    /// Background knowledge, domain understanding, accumulated lore
    Context,
    /// Something noticed — patterns, anomalies, smells, opportunities
    Observation,
    /// Explaining why something exists as-is (post-hoc justification)
    Rationale,
}

impl fmt::Display for PickledKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PickledKind::Decision => write!(f, "decision"),
            PickledKind::Reversal => write!(f, "reversal"),
            PickledKind::Context => write!(f, "context"),
            PickledKind::Observation => write!(f, "observation"),
            PickledKind::Rationale => write!(f, "rationale"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickledTag {
    /// Structure, design, interfaces, data model, patterns
    Architecture,
    /// Speed, memory, efficiency tradeoffs
    Performance,
    /// Error handling, resilience, fault tolerance, recovery
    Reliability,
    /// Access control, threats, trust boundaries
    Security,
    /// Test strategy, coverage, fixture design
    Testing,
    /// Business logic, domain concepts, integration points
    Domain,
    /// Technical debt, conventions, cleanup plans
    Debt,
    /// Dependencies, build, CI/CD, developer experience
    Tooling,
}

impl fmt::Display for PickledTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PickledTag::Architecture => write!(f, "architecture"),
            PickledTag::Performance => write!(f, "performance"),
            PickledTag::Reliability => write!(f, "reliability"),
            PickledTag::Security => write!(f, "security"),
            PickledTag::Testing => write!(f, "testing"),
            PickledTag::Domain => write!(f, "domain"),
            PickledTag::Debt => write!(f, "debt"),
            PickledTag::Tooling => write!(f, "tooling"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChangeRequest {
    pub id: String,
    pub from: String,
    pub target: Value,
    pub change_type: String,
    pub status: String,
    pub priority: Option<String>,
    pub created: String,
    pub description: String,
}

/// Agent registry derived from repo.bog subsystem/skimsystem declarations.
#[derive(Debug)]
pub struct DerivedAgents {
    pub roles: HashMap<String, AgentRole>,
    pub descriptions: HashMap<String, String>,
}

/// Derive agent information from a parsed repo.bog.
///
/// Every subsystem owner becomes a Subsystem agent.
/// Every skimsystem owner becomes a Skimsystem agent.
pub fn derive_agents(repo_bog: &BogFile) -> DerivedAgents {
    let mut roles: HashMap<String, AgentRole> = HashMap::new();
    let mut descriptions: HashMap<String, String> = HashMap::new();

    for ann in &repo_bog.annotations {
        match ann {
            Annotation::Subsystem(s) => {
                roles.entry(s.owner.clone()).or_insert(AgentRole::Subsystem);
                let desc = descriptions.entry(s.owner.clone()).or_default();
                if !desc.is_empty() {
                    desc.push_str(", ");
                }
                desc.push_str(&format!("subsystem: {}", s.name));
            }
            Annotation::Skimsystem(sk) => {
                roles.entry(sk.owner.clone()).or_insert(AgentRole::Skimsystem);
                let desc = descriptions.entry(sk.owner.clone()).or_default();
                if !desc.is_empty() {
                    desc.push_str(", ");
                }
                desc.push_str(&format!("skimsystem: {}", sk.name));
            }
            _ => {}
        }
    }

    DerivedAgents { roles, descriptions }
}
