//! The tools' names, descriptions and arguments, and the check of a call's arguments.

use serde_json::{json, Map, Value};

#[derive(Clone, Copy)]
enum Ty {
    Str,
    List,
}

type Field = (&'static str, Ty);

const ADDR: &[Field] = &[
    ("address1", Ty::Str),
    ("address2", Ty::Str),
    ("city", Ty::Str),
    ("state", Ty::Str),
    ("country", Ty::Str),
    ("zip", Ty::Str),
];

/// A tool: its name, whether it writes, its description and its arguments (in schema order,
/// followed by the address fields when `address`).
struct Tool {
    name: &'static str,
    write: bool,
    description: &'static str,
    args: &'static [Field],
    address: bool,
}

const fn tool(
    name: &'static str,
    write: bool,
    description: &'static str,
    args: &'static [Field],
) -> Tool {
    Tool {
        name,
        write,
        description,
        args,
        address: false,
    }
}

const fn with_address(t: Tool) -> Tool {
    Tool { address: true, ..t }
}

const ITEMS: &[Field] = &[
    ("order_id", Ty::Str),
    ("item_ids", Ty::List),
    ("new_item_ids", Ty::List),
    ("payment_method_id", Ty::Str),
];

/// Every tool, reads first.
const TOOLS: &[Tool] = &[
    tool(
        "calculate",
        false,
        "Evaluate an arithmetic expression and return the result rounded to 2 decimals.",
        &[("expression", Ty::Str)],
    ),
    tool(
        "find_user_id_by_name_zip",
        false,
        "Find a user id by first name, last name and zip code.",
        &[
            ("first_name", Ty::Str),
            ("last_name", Ty::Str),
            ("zip", Ty::Str),
        ],
    ),
    tool(
        "find_user_id_by_email",
        false,
        "Find a user id by email address.",
        &[("email", Ty::Str)],
    ),
    tool(
        "get_order_details",
        false,
        "Get the details of an order (order ids start with '#').",
        &[("order_id", Ty::Str)],
    ),
    tool(
        "get_product_details",
        false,
        "Get the details of a product (product ids differ from item ids).",
        &[("product_id", Ty::Str)],
    ),
    tool(
        "get_item_details",
        false,
        "Get the details of a product item (variant) by item id.",
        &[("item_id", Ty::Str)],
    ),
    tool(
        "get_user_details",
        false,
        "Get the details of a user, including payment methods and orders.",
        &[("user_id", Ty::Str)],
    ),
    tool(
        "list_all_product_types",
        false,
        "List every product type with its product id.",
        &[],
    ),
    tool(
        "transfer_to_human_agents",
        false,
        "Transfer the customer to a human agent with a summary of the issue.",
        &[("summary", Ty::Str)],
    ),
    tool(
        "cancel_pending_order",
        true,
        "Cancel a pending order; reason is 'no longer needed' or 'ordered by mistake'.",
        &[("order_id", Ty::Str), ("reason", Ty::Str)],
    ),
    tool(
        "exchange_delivered_order_items",
        true,
        "Request an exchange of delivered order items for new items of the same product.",
        ITEMS,
    ),
    with_address(tool(
        "modify_pending_order_address",
        true,
        "Change the shipping address of a pending order.",
        &[("order_id", Ty::Str)],
    )),
    tool(
        "modify_pending_order_items",
        true,
        "Replace items of a pending order with other variants of the same product (one-time).",
        ITEMS,
    ),
    tool(
        "modify_pending_order_payment",
        true,
        "Change the payment method of a pending order.",
        &[("order_id", Ty::Str), ("payment_method_id", Ty::Str)],
    ),
    with_address(tool(
        "modify_user_address",
        true,
        "Change the default address of a user.",
        &[("user_id", Ty::Str)],
    )),
    tool(
        "return_delivered_order_items",
        true,
        "Request a return of delivered order items.",
        &[
            ("order_id", Ty::Str),
            ("item_ids", Ty::List),
            ("payment_method_id", Ty::Str),
        ],
    ),
];

impl Tool {
    /// The arguments, in the order they are listed in the schema.
    fn fields(&self) -> impl Iterator<Item = &'static Field> {
        let address: &'static [Field] = if self.address { ADDR } else { &[] };
        self.args.iter().chain(address)
    }
}

fn find(name: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|t| t.name == name)
}

/// Every tool's name, reads first.
pub fn tool_names() -> impl Iterator<Item = &'static str> {
    TOOLS.iter().map(|t| t.name)
}

/// True for a tool that changes the database.
pub fn is_write(name: &str) -> bool {
    find(name).is_some_and(|t| t.write)
}

fn property(ty: Ty) -> Value {
    match ty {
        Ty::Str => json!({"type": "string"}),
        Ty::List => json!({"type": "array", "items": {"type": "string"}}),
    }
}

/// MCP-style tool entries `{name, description, inputSchema}`.
pub fn tool_schemas() -> Vec<Value> {
    TOOLS
        .iter()
        .map(|t| {
            let props: Map<String, Value> = t
                .fields()
                .map(|(a, ty)| (a.to_string(), property(*ty)))
                .collect();
            let required: Vec<&str> = t.fields().map(|(a, _)| *a).collect();
            json!({"name": t.name, "description": t.description,
                   "inputSchema": {"type": "object", "properties": props, "required": required}})
        })
        .collect()
}

/// One argument against its declared type.
fn check_arg(name: &str, k: &str, ty: Ty, v: Option<&Value>) -> Result<(), String> {
    match (v, ty) {
        (None, _) => Err(format!("{name}() missing required argument: '{k}'")),
        (Some(Value::String(_)), Ty::Str) => Ok(()),
        (Some(Value::Array(a)), Ty::List) if a.iter().all(Value::is_string) => Ok(()),
        (Some(_), Ty::Str) => Err(format!("{name}() argument '{k}' must be a string")),
        (Some(_), Ty::List) => Err(format!("{name}() argument '{k}' must be a list of strings")),
    }
}

/// Checks a call the way Python would check its signature; the message is the error to report.
pub(super) fn check_call(name: &str, args: &Value) -> Result<(), String> {
    let Some(tool) = find(name) else {
        return Err(format!("Unknown tool {name}"));
    };
    let params: Vec<&Field> = tool.fields().collect();
    let Some(obj) = args.as_object() else {
        return Err(format!("{name}() arguments must be a JSON object"));
    };
    for (k, ty) in &params {
        check_arg(name, k, *ty, obj.get(*k))?;
    }
    match obj.keys().find(|k| !params.iter().any(|(p, _)| p == k)) {
        Some(k) => Err(format!("{name}() got an unexpected keyword argument '{k}'")),
        None => Ok(()),
    }
}
