//! The tools' names, descriptions and arguments, and the check of a call's arguments.

use serde_json::{json, Map, Value};

pub const READ_TOOLS: &[&str] = &[
    "calculate",
    "find_user_id_by_name_zip",
    "find_user_id_by_email",
    "get_order_details",
    "get_product_details",
    "get_item_details",
    "get_user_details",
    "list_all_product_types",
    "transfer_to_human_agents",
];
pub const WRITE_TOOLS: &[&str] = &[
    "cancel_pending_order",
    "exchange_delivered_order_items",
    "modify_pending_order_address",
    "modify_pending_order_items",
    "modify_pending_order_payment",
    "modify_user_address",
    "return_delivered_order_items",
];

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

const DESCRIPTIONS: &[(&str, &str)] = &[
    (
        "calculate",
        "Evaluate an arithmetic expression and return the result rounded to 2 decimals.",
    ),
    (
        "find_user_id_by_name_zip",
        "Find a user id by first name, last name and zip code.",
    ),
    ("find_user_id_by_email", "Find a user id by email address."),
    (
        "get_order_details",
        "Get the details of an order (order ids start with '#').",
    ),
    (
        "get_product_details",
        "Get the details of a product (product ids differ from item ids).",
    ),
    (
        "get_item_details",
        "Get the details of a product item (variant) by item id.",
    ),
    (
        "get_user_details",
        "Get the details of a user, including payment methods and orders.",
    ),
    (
        "list_all_product_types",
        "List every product type with its product id.",
    ),
    (
        "transfer_to_human_agents",
        "Transfer the customer to a human agent with a summary of the issue.",
    ),
    (
        "cancel_pending_order",
        "Cancel a pending order; reason is 'no longer needed' or 'ordered by mistake'.",
    ),
    (
        "exchange_delivered_order_items",
        "Request an exchange of delivered order items for new items of the same product.",
    ),
    (
        "modify_pending_order_address",
        "Change the shipping address of a pending order.",
    ),
    (
        "modify_pending_order_items",
        "Replace items of a pending order with other variants of the same product (one-time).",
    ),
    (
        "modify_pending_order_payment",
        "Change the payment method of a pending order.",
    ),
    (
        "modify_user_address",
        "Change the default address of a user.",
    ),
    (
        "return_delivered_order_items",
        "Request a return of delivered order items.",
    ),
];

/// The arguments of a tool, in the order they are listed in its schema.
fn fields(name: &str) -> Vec<Field> {
    use Ty::*;
    let with_addr = |first: Field| [&[first], ADDR].concat();
    match name {
        "calculate" => vec![("expression", Str)],
        "find_user_id_by_name_zip" => vec![("first_name", Str), ("last_name", Str), ("zip", Str)],
        "find_user_id_by_email" => vec![("email", Str)],
        "get_order_details" => vec![("order_id", Str)],
        "modify_pending_order_payment" => vec![("order_id", Str), ("payment_method_id", Str)],
        "get_product_details" => vec![("product_id", Str)],
        "get_item_details" => vec![("item_id", Str)],
        "get_user_details" => vec![("user_id", Str)],
        "transfer_to_human_agents" => vec![("summary", Str)],
        "cancel_pending_order" => vec![("order_id", Str), ("reason", Str)],
        "exchange_delivered_order_items" | "modify_pending_order_items" => vec![
            ("order_id", Str),
            ("item_ids", List),
            ("new_item_ids", List),
            ("payment_method_id", Str),
        ],
        "modify_pending_order_address" => with_addr(("order_id", Str)),
        "modify_user_address" => with_addr(("user_id", Str)),
        "return_delivered_order_items" => vec![
            ("order_id", Str),
            ("item_ids", List),
            ("payment_method_id", Str),
        ],
        _ => vec![],
    }
}

fn spec(name: &str) -> Option<(&'static str, Vec<Field>)> {
    let (_, desc) = DESCRIPTIONS.iter().find(|(n, _)| *n == name)?;
    Some((desc, fields(name)))
}

fn property(ty: Ty) -> Value {
    match ty {
        Ty::Str => json!({"type": "string"}),
        Ty::List => json!({"type": "array", "items": {"type": "string"}}),
    }
}

/// MCP-style tool entries `{name, description, inputSchema}`.
pub fn tool_schemas() -> Vec<Value> {
    READ_TOOLS
        .iter()
        .chain(WRITE_TOOLS)
        .map(|name| {
            let (desc, args) = spec(name).expect("every tool has a spec");
            let props: Map<String, Value> = args
                .iter()
                .map(|(a, ty)| (a.to_string(), property(*ty)))
                .collect();
            let required: Vec<&str> = args.iter().map(|(a, _)| *a).collect();
            json!({"name": name, "description": desc,
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
    let Some((_, params)) = spec(name) else {
        return Err(format!("Unknown tool {name}"));
    };
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
