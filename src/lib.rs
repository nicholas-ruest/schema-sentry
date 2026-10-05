#![forbid(unsafe_code)]

use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::BTreeSet;

/// The severity of a detected schema change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Breaking,
    Warning,
}

/// One compatibility finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub severity: Severity,
    pub path: String,
    pub rule: String,
    pub message: String,
}

/// The complete result of comparing an old schema to a proposed new schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    pub compatible: bool,
    pub breaking_changes: usize,
    pub warnings: usize,
    pub findings: Vec<Finding>,
}

/// Compare JSON Schemas for backward input compatibility.
///
/// A compatible new schema accepts every instance accepted by the old schema
/// for the supported keyword subset. Unknown composition changes are reported
/// conservatively rather than silently ignored.
#[must_use]
pub fn compare(old: &Value, new: &Value) -> Report {
    let mut findings = Vec::new();
    compare_at(old, new, "$", &mut findings);
    findings.sort_by(|left, right| left.path.cmp(&right.path).then(left.rule.cmp(&right.rule)));

    let breaking_changes = findings
        .iter()
        .filter(|finding| finding.severity == Severity::Breaking)
        .count();
    let warnings = findings.len() - breaking_changes;

    Report {
        compatible: breaking_changes == 0,
        breaking_changes,
        warnings,
        findings,
    }
}

fn compare_at(old: &Value, new: &Value, path: &str, findings: &mut Vec<Finding>) {
    match (old, new) {
        (Value::Bool(false), _) | (_, Value::Bool(true)) => {}
        (Value::Bool(true), Value::Bool(false)) => {
            breaking(
                findings,
                path,
                "schema_forbidden",
                "new schema rejects every value",
            );
        }
        (Value::Bool(true), Value::Object(_)) => {
            breaking(
                findings,
                path,
                "schema_restricted",
                "unconstrained old schema became constrained",
            );
        }
        (Value::Object(_), Value::Bool(false)) => {
            breaking(
                findings,
                path,
                "schema_forbidden",
                "new schema rejects every value",
            );
        }
        (Value::Object(old_map), Value::Object(new_map)) => {
            compare_objects(old_map, new_map, path, findings);
        }
        _ => warning(
            findings,
            path,
            "invalid_schema_shape",
            "schema must be a JSON object or boolean",
        ),
    }
}

fn compare_objects(
    old: &Map<String, Value>,
    new: &Map<String, Value>,
    path: &str,
    findings: &mut Vec<Finding>,
) {
    compare_types(old, new, path, findings);
    compare_enum(old, new, path, findings);
    compare_required(old, new, path, findings);
    compare_properties(old, new, path, findings);

    compare_numeric_bounds(old, new, path, findings);
    compare_lower_bound(old, new, "minLength", path, findings);
    compare_upper_bound(old, new, "maxLength", path, findings);
    compare_lower_bound(old, new, "minItems", path, findings);
    compare_upper_bound(old, new, "maxItems", path, findings);
    compare_lower_bound(old, new, "minProperties", path, findings);
    compare_upper_bound(old, new, "maxProperties", path, findings);

    compare_restrictive_value(old, new, "pattern", path, findings);
    compare_restrictive_value(old, new, "multipleOf", path, findings);
    compare_true_constraint(old, new, "uniqueItems", path, findings);

    for keyword in [
        "allOf",
        "anyOf",
        "oneOf",
        "not",
        "if",
        "then",
        "else",
        "$ref",
        "$dynamicRef",
        "dependentRequired",
        "dependentSchemas",
        "patternProperties",
        "propertyNames",
        "prefixItems",
        "contains",
        "minContains",
        "maxContains",
        "unevaluatedProperties",
        "unevaluatedItems",
    ] {
        if old.get(keyword) != new.get(keyword) && new.contains_key(keyword) {
            breaking(
                findings,
                path,
                "composition_changed",
                format!("'{keyword}' was added or changed; compatibility cannot be proven"),
            );
        }
    }

    if old.get("format") != new.get("format") && new.contains_key("format") {
        warning(
            findings,
            path,
            "format_changed",
            "format was added or changed; enforcement depends on the validator",
        );
    }
}

fn compare_types(
    old: &Map<String, Value>,
    new: &Map<String, Value>,
    path: &str,
    findings: &mut Vec<Finding>,
) {
    let old_types = type_set(old.get("type"));
    let new_types = type_set(new.get("type"));

    let narrowed = match (&old_types, &new_types) {
        (_, None) => false,
        (None, Some(_)) => true,
        (Some(old_set), Some(new_set)) => old_set
            .iter()
            .any(|old_type| !type_is_accepted(old_type, new_set)),
    };

    if narrowed {
        breaking(
            findings,
            path,
            "type_narrowed",
            format!(
                "accepted types narrowed from {} to {}",
                display_types(&old_types),
                display_types(&new_types)
            ),
        );
    }
}

fn type_is_accepted(old_type: &str, new_types: &BTreeSet<String>) -> bool {
    new_types.contains(old_type) || (old_type == "integer" && new_types.contains("number"))
}

fn type_set(value: Option<&Value>) -> Option<BTreeSet<String>> {
    match value {
        None => None,
        Some(Value::String(kind)) => Some([kind.clone()].into_iter().collect()),
        Some(Value::Array(kinds)) => Some(
            kinds
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect(),
        ),
        Some(_) => Some(BTreeSet::new()),
    }
}

fn display_types(types: &Option<BTreeSet<String>>) -> String {
    match types {
        None => "any".to_owned(),
        Some(types) if types.is_empty() => "an invalid type declaration".to_owned(),
        Some(types) => types.iter().cloned().collect::<Vec<_>>().join(" | "),
    }
}

fn compare_enum(
    old: &Map<String, Value>,
    new: &Map<String, Value>,
    path: &str,
    findings: &mut Vec<Finding>,
) {
    let old_values = allowed_values(old);
    let new_values = allowed_values(new);

    let removed = match (&old_values, &new_values) {
        (_, None) => Vec::new(),
        (None, Some(_)) => vec!["previously unconstrained values".to_owned()],
        (Some(old_values), Some(new_values)) => old_values
            .iter()
            .filter(|old_value| !new_values.contains(old_value))
            .map(compact_json)
            .collect(),
    };

    if !removed.is_empty() {
        breaking(
            findings,
            path,
            "allowed_values_narrowed",
            format!("new enum/const rejects: {}", removed.join(", ")),
        );
    }
}

fn allowed_values(schema: &Map<String, Value>) -> Option<Vec<Value>> {
    if let Some(value) = schema.get("const") {
        return Some(vec![value.clone()]);
    }
    schema.get("enum").and_then(Value::as_array).cloned()
}

fn compare_required(
    old: &Map<String, Value>,
    new: &Map<String, Value>,
    path: &str,
    findings: &mut Vec<Finding>,
) {
    let old_required = string_set(old.get("required"));
    let new_required = string_set(new.get("required"));

    for property in new_required.difference(&old_required) {
        breaking(
            findings,
            &property_path(path, property),
            "property_became_required",
            format!("property '{property}' is newly required"),
        );
    }
}

fn string_set(value: Option<&Value>) -> BTreeSet<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect()
}

fn compare_properties(
    old: &Map<String, Value>,
    new: &Map<String, Value>,
    path: &str,
    findings: &mut Vec<Finding>,
) {
    let old_properties = old
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let new_properties = new
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let old_additional = old
        .get("additionalProperties")
        .unwrap_or(&Value::Bool(true));
    let new_additional = new
        .get("additionalProperties")
        .unwrap_or(&Value::Bool(true));

    for (name, old_property) in &old_properties {
        let nested_path = property_path(path, name);
        if let Some(new_property) = new_properties.get(name) {
            compare_at(old_property, new_property, &nested_path, findings);
        } else if new_additional == &Value::Bool(false) {
            breaking(
                findings,
                &nested_path,
                "property_no_longer_allowed",
                format!("property '{name}' is no longer accepted"),
            );
        } else if new_additional.is_object() {
            compare_at(old_property, new_additional, &nested_path, findings);
        }
    }

    for (name, new_property) in &new_properties {
        if old_properties.contains_key(name) || old_additional == &Value::Bool(false) {
            continue;
        }
        compare_at(
            old_additional,
            new_property,
            &property_path(path, name),
            findings,
        );
    }

    compare_additional_properties(old_additional, new_additional, path, findings);

    let old_items = old.get("items").unwrap_or(&Value::Bool(true));
    let new_items = new.get("items").unwrap_or(&Value::Bool(true));
    compare_at(old_items, new_items, &format!("{path}[]"), findings);
}

#[derive(Clone, Copy)]
struct Bound {
    value: f64,
    exclusive: bool,
}

fn compare_numeric_bounds(
    old: &Map<String, Value>,
    new: &Map<String, Value>,
    path: &str,
    findings: &mut Vec<Finding>,
) {
    let old_lower = lower_bound(old);
    let new_lower = lower_bound(new);
    if bound_is_tighter(old_lower, new_lower, true) {
        breaking(
            findings,
            path,
            "lower_bound_raised",
            format!(
                "numeric lower bound tightened from {} to {}",
                display_bound(old_lower),
                display_bound(new_lower)
            ),
        );
    }

    let old_upper = upper_bound(old);
    let new_upper = upper_bound(new);
    if bound_is_tighter(old_upper, new_upper, false) {
        breaking(
            findings,
            path,
            "upper_bound_lowered",
            format!(
                "numeric upper bound tightened from {} to {}",
                display_bound(old_upper),
                display_bound(new_upper)
            ),
        );
    }
}

fn lower_bound(schema: &Map<String, Value>) -> Option<Bound> {
    strongest_bound(
        schema
            .get("minimum")
            .and_then(Value::as_f64)
            .map(|value| Bound {
                value,
                exclusive: false,
            }),
        schema
            .get("exclusiveMinimum")
            .and_then(Value::as_f64)
            .map(|value| Bound {
                value,
                exclusive: true,
            }),
        true,
    )
}

fn upper_bound(schema: &Map<String, Value>) -> Option<Bound> {
    strongest_bound(
        schema
            .get("maximum")
            .and_then(Value::as_f64)
            .map(|value| Bound {
                value,
                exclusive: false,
            }),
        schema
            .get("exclusiveMaximum")
            .and_then(Value::as_f64)
            .map(|value| Bound {
                value,
                exclusive: true,
            }),
        false,
    )
}

fn strongest_bound(first: Option<Bound>, second: Option<Bound>, lower: bool) -> Option<Bound> {
    match (first, second) {
        (Some(first), Some(second)) => {
            let first_is_stronger = if lower {
                first.value > second.value
            } else {
                first.value < second.value
            };
            if first.value == second.value {
                Some(Bound {
                    value: first.value,
                    exclusive: first.exclusive || second.exclusive,
                })
            } else if first_is_stronger {
                Some(first)
            } else {
                Some(second)
            }
        }
        (bound @ Some(_), None) | (None, bound @ Some(_)) => bound,
        (None, None) => None,
    }
}

fn bound_is_tighter(old: Option<Bound>, new: Option<Bound>, lower: bool) -> bool {
    match (old, new) {
        (_, None) => false,
        (None, Some(_)) => true,
        (Some(old), Some(new)) => {
            let value_is_tighter = if lower {
                new.value > old.value
            } else {
                new.value < old.value
            };
            value_is_tighter || (new.value == old.value && new.exclusive && !old.exclusive)
        }
    }
}

fn display_bound(bound: Option<Bound>) -> String {
    match bound {
        None => "unbounded".to_owned(),
        Some(Bound { value, exclusive }) => {
            let suffix = if exclusive { " (exclusive)" } else { "" };
            format!("{value}{suffix}")
        }
    }
}

fn compare_additional_properties(
    old: &Value,
    new: &Value,
    path: &str,
    findings: &mut Vec<Finding>,
) {
    match (old, new) {
        (Value::Bool(true), Value::Bool(false)) | (Value::Object(_), Value::Bool(false)) => {
            breaking(
                findings,
                path,
                "additional_properties_forbidden",
                "additional properties were allowed before but are now forbidden",
            );
        }
        (Value::Bool(true), Value::Object(_)) => breaking(
            findings,
            path,
            "additional_properties_restricted",
            "additional properties were unconstrained but must now match a schema",
        ),
        (Value::Object(old_schema), Value::Object(new_schema)) => compare_at(
            &Value::Object(old_schema.clone()),
            &Value::Object(new_schema.clone()),
            &format!("{path}.*"),
            findings,
        ),
        _ => {}
    }
}

fn compare_lower_bound(
    old: &Map<String, Value>,
    new: &Map<String, Value>,
    keyword: &str,
    path: &str,
    findings: &mut Vec<Finding>,
) {
    let old_value = old.get(keyword).and_then(Value::as_f64);
    let new_value = new.get(keyword).and_then(Value::as_f64);
    if let Some(new_value) = new_value
        && old_value.is_none_or(|old_value| new_value > old_value)
    {
        breaking(
            findings,
            path,
            "lower_bound_raised",
            format!("'{keyword}' was raised from {old_value:?} to {new_value}"),
        );
    }
}

fn compare_upper_bound(
    old: &Map<String, Value>,
    new: &Map<String, Value>,
    keyword: &str,
    path: &str,
    findings: &mut Vec<Finding>,
) {
    let old_value = old.get(keyword).and_then(Value::as_f64);
    let new_value = new.get(keyword).and_then(Value::as_f64);
    if let Some(new_value) = new_value
        && old_value.is_none_or(|old_value| new_value < old_value)
    {
        breaking(
            findings,
            path,
            "upper_bound_lowered",
            format!("'{keyword}' was lowered from {old_value:?} to {new_value}"),
        );
    }
}

fn compare_restrictive_value(
    old: &Map<String, Value>,
    new: &Map<String, Value>,
    keyword: &str,
    path: &str,
    findings: &mut Vec<Finding>,
) {
    if old.get(keyword) != new.get(keyword) && new.contains_key(keyword) {
        breaking(
            findings,
            path,
            "constraint_added_or_changed",
            format!("'{keyword}' was added or changed"),
        );
    }
}

fn compare_true_constraint(
    old: &Map<String, Value>,
    new: &Map<String, Value>,
    keyword: &str,
    path: &str,
    findings: &mut Vec<Finding>,
) {
    if new.get(keyword) == Some(&Value::Bool(true)) && old.get(keyword) != new.get(keyword) {
        breaking(
            findings,
            path,
            "constraint_enabled",
            format!("'{keyword}' is newly enabled"),
        );
    }
}

fn property_path(parent: &str, property: &str) -> String {
    if property
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        format!("{parent}.{property}")
    } else {
        format!(
            "{parent}[{}]",
            compact_json(&Value::String(property.to_owned()))
        )
    }
}

fn compact_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "<unprintable>".to_owned())
}

fn breaking(findings: &mut Vec<Finding>, path: &str, rule: &str, message: impl Into<String>) {
    findings.push(Finding {
        severity: Severity::Breaking,
        path: path.to_owned(),
        rule: rule.to_owned(),
        message: message.into(),
    });
}

fn warning(findings: &mut Vec<Finding>, path: &str, rule: &str, message: impl Into<String>) {
    findings.push(Finding {
        severity: Severity::Warning,
        path: path.to_owned(),
        rule: rule.to_owned(),
        message: message.into(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn identical_schemas_are_compatible() {
        let schema =
            json!({"type":"object","required":["id"],"properties":{"id":{"type":"string"}}});
        assert!(compare(&schema, &schema).compatible);
    }

    #[test]
    fn newly_required_property_breaks() {
        let old = json!({"type":"object","properties":{"mode":{"type":"string"}}});
        let new =
            json!({"type":"object","required":["mode"],"properties":{"mode":{"type":"string"}}});
        let report = compare(&old, &new);
        assert!(!report.compatible);
        assert_eq!(report.findings[0].rule, "property_became_required");
        assert_eq!(report.findings[0].path, "$.mode");
    }

    #[test]
    fn removed_required_property_is_compatible() {
        let old = json!({"type":"object","required":["mode"]});
        let new = json!({"type":"object"});
        assert!(compare(&old, &new).compatible);
    }

    #[test]
    fn type_narrowing_breaks_but_widening_does_not() {
        let string_or_null = json!({"type":["string","null"]});
        let string = json!({"type":"string"});
        assert!(!compare(&string_or_null, &string).compatible);
        assert!(compare(&string, &string_or_null).compatible);
    }

    #[test]
    fn integer_is_a_subset_of_number() {
        assert!(compare(&json!({"type":"integer"}), &json!({"type":"number"})).compatible);
        assert!(!compare(&json!({"type":"number"}), &json!({"type":"integer"})).compatible);
    }

    #[test]
    fn removing_enum_member_breaks() {
        let old = json!({"enum":["fast","safe"]});
        let new = json!({"enum":["safe","audit"]});
        let report = compare(&old, &new);
        assert!(!report.compatible);
        assert!(report.findings[0].message.contains("fast"));
    }

    #[test]
    fn adding_enum_member_is_compatible() {
        let old = json!({"enum":["safe"]});
        let new = json!({"enum":["safe","audit"]});
        assert!(compare(&old, &new).compatible);
    }

    #[test]
    fn nested_constraint_is_reported_at_property_path() {
        let old = json!({"properties":{"query":{"type":"string","maxLength":100}}});
        let new = json!({"properties":{"query":{"type":"string","maxLength":20}}});
        let report = compare(&old, &new);
        assert_eq!(report.findings[0].path, "$.query");
        assert_eq!(report.findings[0].rule, "upper_bound_lowered");
    }

    #[test]
    fn forbidding_additional_properties_breaks() {
        let old = json!({"type":"object"});
        let new = json!({"type":"object","additionalProperties":false});
        assert_eq!(compare(&old, &new).breaking_changes, 1);
    }

    #[test]
    fn removed_declared_property_breaks_when_extras_are_forbidden() {
        let old = json!({"properties":{"legacy":{"type":"string"}}});
        let new = json!({"properties":{},"additionalProperties":false});
        let report = compare(&old, &new);
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.rule == "property_no_longer_allowed")
        );
    }

    #[test]
    fn newly_declared_property_cannot_narrow_old_additional_values() {
        let old = json!({"type":"object"});
        let new = json!({"type":"object","properties":{"mode":{"type":"string"}}});
        let report = compare(&old, &new);
        assert!(!report.compatible);
        assert_eq!(report.findings[0].path, "$.mode");
        assert_eq!(report.findings[0].rule, "schema_restricted");
    }

    #[test]
    fn property_added_to_closed_schema_is_compatible() {
        let old = json!({"type":"object","additionalProperties":false});
        let new = json!({"type":"object","properties":{"mode":{"type":"string"}},"additionalProperties":false});
        assert!(compare(&old, &new).compatible);
    }

    #[test]
    fn tightening_items_breaks() {
        let old = json!({"type":"array","items":{"type":["string","number"]}});
        let new = json!({"type":"array","items":{"type":"string"}});
        let report = compare(&old, &new);
        assert_eq!(report.findings[0].path, "$[]");
    }

    #[test]
    fn format_change_is_only_a_warning() {
        let old = json!({"type":"string"});
        let new = json!({"type":"string","format":"email"});
        let report = compare(&old, &new);
        assert!(report.compatible);
        assert_eq!(report.warnings, 1);
    }

    #[test]
    fn boolean_schema_rules_are_respected() {
        assert!(compare(&json!(false), &json!({"type":"string"})).compatible);
        assert!(compare(&json!({"type":"string"}), &json!(true)).compatible);
        assert!(!compare(&json!(true), &json!({"type":"string"})).compatible);
        assert!(!compare(&json!({"type":"string"}), &json!(false)).compatible);
    }

    #[test]
    fn changed_composition_is_conservatively_breaking() {
        let old = json!({"type":"string"});
        let new = json!({"type":"string","oneOf":[{"pattern":"^a"},{"pattern":"^b"}]});
        assert_eq!(compare(&old, &new).findings[0].rule, "composition_changed");
    }

    #[test]
    fn inclusive_bound_becoming_exclusive_breaks() {
        let old = json!({"type":"number","minimum":0});
        let new = json!({"type":"number","exclusiveMinimum":0});
        assert!(!compare(&old, &new).compatible);
    }

    #[test]
    fn unusual_property_names_use_bracket_paths() {
        let old = json!({"properties":{"user-name":{"type":"string"}}});
        let new = json!({"properties":{"user-name":{"type":"number"}}});
        assert_eq!(compare(&old, &new).findings[0].path, "$[\"user-name\"]");
    }
}
