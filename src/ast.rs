use std::collections::HashMap;
use std::fmt;

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
    Policies(PoliciesAnnotation),
    ChangeRequests(Vec<ChangeRequest>),
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
}

#[derive(Debug, Clone)]
pub struct PoliciesAnnotation {
    pub fields: HashMap<String, Value>,
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
