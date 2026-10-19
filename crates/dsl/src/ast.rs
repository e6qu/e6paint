//! The parsed mol document: blocks of typed statements.

/// Block families of DSL wave 1 + 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    /// Directed flowchart (layered layout).
    Flow,
    /// State machine (layered layout).
    State,
    /// Mindmap (tree layout, `>` links).
    Mind,
    /// Sequence diagram (lane layout, `->` messages).
    Seq,
    /// Entity-relationship (layered layout, member lists).
    Er,
    /// UML class (layered layout, member lists + `<|--`).
    Class,
    /// Timeline (stacked rows).
    Timeline,
    /// Pie chart (sector geometry from values).
    Pie,
    /// Gantt (bars on a day axis).
    Gantt,
    /// Git history (branch lanes).
    Git,
}

impl BlockKind {
    /// The header keyword that selects this block kind.
    pub fn keyword(self) -> &'static str {
        match self {
            Self::Flow => "flow",
            Self::State => "state",
            Self::Mind => "mind",
            Self::Seq => "seq",
            Self::Er => "er",
            Self::Class => "class",
            Self::Timeline => "timeline",
            Self::Pie => "pie",
            Self::Gantt => "gantt",
            Self::Git => "git",
        }
    }

    pub fn from_keyword(keyword: &str) -> Option<Self> {
        match keyword {
            "flow" => Some(Self::Flow),
            "state" => Some(Self::State),
            "mind" => Some(Self::Mind),
            "seq" => Some(Self::Seq),
            "er" => Some(Self::Er),
            "class" => Some(Self::Class),
            "timeline" => Some(Self::Timeline),
            "pie" => Some(Self::Pie),
            "gantt" => Some(Self::Gantt),
            "git" => Some(Self::Git),
            _ => None,
        }
    }
}

/// Layout direction for layered blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Layered left→right (default for flow/state).
    LeftRight,
    /// Layered top→bottom.
    TopDown,
}

/// A parsed mol document: top-level blocks in source order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MolDocument {
    pub blocks: Vec<MolBlock>,
}

/// One block: kind, optional quoted title, and its statements.
#[derive(Debug, Clone, PartialEq)]
pub struct MolBlock {
    pub kind: BlockKind,
    pub title: Option<String>,
    pub direction: Direction,
    pub lines: Vec<MolLine>,
}

/// One statement line inside a block.
#[derive(Debug, Clone, PartialEq)]
pub enum MolLine {
    /// `id[Label]` — declares or redeclares a node's label.
    NodeDecl { id: String, label: String },
    /// `a --> b`, `a[X] --> b[Y] : edge label`
    Edge {
        from: MolNode,
        to: MolNode,
        label: Option<String>,
    },
    /// `parent > child > grandchild` (mind blocks).
    MindLink { chain: Vec<String> },
    /// `direction: LR` (flow/state blocks).
    Direction { direction: Direction },
    /// `a -> b : message` or `a --> b : return` (seq blocks).
    SeqMsg {
        from: String,
        to: String,
        dashed: bool,
        label: Option<String>,
    },
    /// `id: member, member, …` (er/class/timeline/pie/gantt rows).
    Members { id: String, text: String },
    /// `commit x` / `branch b` / `checkout b` / `merge b` (git blocks).
    GitOp { op: GitOpKind, arg: String },
}

/// Git block operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitOpKind {
    Commit,
    Branch,
    Checkout,
    Merge,
}

impl GitOpKind {
    pub fn keyword(self) -> &'static str {
        match self {
            Self::Commit => "commit",
            Self::Branch => "branch",
            Self::Checkout => "checkout",
            Self::Merge => "merge",
        }
    }

    pub fn from_keyword(keyword: &str) -> Option<Self> {
        match keyword {
            "commit" => Some(Self::Commit),
            "branch" => Some(Self::Branch),
            "checkout" => Some(Self::Checkout),
            "merge" => Some(Self::Merge),
            _ => None,
        }
    }
}

/// A node reference inside an edge: id plus optional inline label.
#[derive(Debug, Clone, PartialEq)]
pub struct MolNode {
    pub id: String,
    pub label: Option<String>,
}
