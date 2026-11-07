//! The digital twin: a benchmark's `view` of a world's current state, in one generic shape any
//! world can draw (areas of items, flows between items, KPIs). See docs/protocol.md, "The world's
//! view".
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// `describe.view`: the benchmark answers `view`, and arenad passes it the `inspect` docs of
/// `docs` (part names) besides the connections.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ViewInfo {
    #[serde(default)]
    pub docs: Vec<String>,
}

/// `view`: the world's connections and, per part `describe.view.docs` names, every doc of its
/// live instance (`{part: {key: doc}}`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewParams {
    pub connections: Map<String, Value>,
    #[serde(default)]
    pub docs: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewResult {
    pub view: TwinView,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TwinView {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock: Option<String>,
    pub areas: Vec<TwinArea>,
    pub flows: Vec<TwinFlow>,
    pub kpis: Vec<TwinKpi>,
}

/// A zone of the world: `kind` is "plant", "lab", "warehouse", "customers", "carriers",
/// "approvals" or another short word.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TwinArea {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub items: Vec<TwinItem>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TwinItem {
    pub id: String,
    pub label: String,
    pub status: String,
    pub abnormal: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qty: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
    /// The id of an item this one points at (an order's allocated lot, say).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}

/// Something moving from one item to another (a booked shipment, lot to customer).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TwinFlow {
    pub id: String,
    pub from: String,
    pub to: String,
    pub label: String,
    pub status: String,
    pub abnormal: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TwinKpi {
    pub id: String,
    pub label: String,
    pub value: String,
    pub abnormal: bool,
}
