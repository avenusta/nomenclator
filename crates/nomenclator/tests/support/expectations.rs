//! Compares corpus expectations with results as subsets. The direct and the wire test
//! suites share this module.

use serde_json::Value;

/// Compares only the fields that `expected` lists. A finding (a map with `status`) also
/// matches a plain expected value (its resolved `value`) or `null` (unknown). Fails with
/// the path of the first difference.
pub fn expect_subset(expected: &Value, actual: &Value, at: &str) -> Result<(), String> {
    let finding_status = actual.get("status").and_then(Value::as_str);
    let expected_is_full_finding = expected.get("status").is_some();
    match (expected, finding_status) {
        (Value::Null, _) if is_absent(actual) => Ok(()),
        (Value::Object(_), None) if expected_is_full_finding => {
            let expected_status = expected.get("status").and_then(Value::as_str);
            match (expected_status, actual.is_null()) {
                (Some("unknown"), true) => Ok(()),
                (Some("resolved"), false) => match expected.get("value") {
                    Some(expected_value) => expect_subset(expected_value, actual, at),
                    None => Ok(()),
                },
                _ => Err(format!(
                    "{at}: expected {}, got {}",
                    show(expected),
                    show(actual)
                )),
            }
        }
        (_, Some(status)) if !expected_is_full_finding => {
            if status != "resolved" {
                return Err(format!("{at}: expected {}, got {status}", show(expected)));
            }
            expect_subset(expected, &actual["value"], at)
        }
        // An empty map as a field value means that nothing was reported. As a list element
        // it matches any one element, so a list of `{}` checks only the count.
        (Value::Object(expected_fields), _) if expected_fields.is_empty() && !at.ends_with(']') => {
            if is_absent(actual) {
                Ok(())
            } else {
                Err(format!("{at}: expected nothing, got {}", show(actual)))
            }
        }
        (Value::Object(expected_fields), _) => {
            for (key, expected_value) in expected_fields {
                let key_name = key.as_str();
                // A result field must be reported, even as unknown. A key inside a reported
                // map, such as one provider in `provider_ids`, may be absent.
                let is_result_field = at == "expect";
                let actual_value = match actual.get(key) {
                    Some(value) => value,
                    None if is_result_field => {
                        return Err(format!("{at}.{key_name}: not reported"));
                    }
                    None => &Value::Null,
                };
                expect_subset(expected_value, actual_value, &format!("{at}.{key_name}"))?;
            }
            Ok(())
        }
        (Value::Array(expected_items), _) => {
            let actual_items = actual.as_array().map(Vec::as_slice).unwrap_or_default();
            if expected_items.len() != actual_items.len() {
                return Err(format!(
                    "{at}: expected {}, got {}",
                    show(expected),
                    show(actual)
                ));
            }
            for (index, (expected_item, actual_item)) in
                expected_items.iter().zip(actual_items).enumerate()
            {
                expect_subset(expected_item, actual_item, &format!("{at}[{index}]"))?;
            }
            Ok(())
        }
        _ if expected == actual => Ok(()),
        _ => Err(format!(
            "{at}: expected {}, got {}",
            show(expected),
            show(actual)
        )),
    }
}

/// Returns `true` if `actual` is null, an unknown finding, or an empty map or list.
fn is_absent(actual: &Value) -> bool {
    match actual {
        Value::Null => true,
        Value::Object(fields) => {
            fields.is_empty() || actual.get("status").and_then(Value::as_str) == Some("unknown")
        }
        Value::Array(items) => items.is_empty(),
        _ => false,
    }
}

fn show(value: &Value) -> String {
    serde_json::to_string(value)
        .unwrap_or_default()
        .trim()
        .replace('\n', " ")
}
