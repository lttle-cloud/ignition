use anyhow::{Result, bail};
use ignition::constants::DEFAULT_NAMESPACE;
use serde_yaml::{Mapping, Sequence, Value};

use crate::expr::ctx::ExprEvalContext;

pub fn eval_expr(expr: &str, context: &ExprEvalContext) -> Result<Value> {
    let program = cel::Program::compile(expr)?;

    let value = program.execute(&context.try_into()?)?;
    let value = match value {
        cel::Value::Bool(b) => Value::Bool(b),
        cel::Value::UInt(n) => Value::Number(n.into()),
        cel::Value::Int(n) => Value::Number(n.into()),
        cel::Value::String(s) => Value::String(s.to_string()),
        cel::Value::Null => Value::Null,
        _ => bail!("Invalid value returned by expression '{}'", expr),
    };

    return Ok(value);
}

pub fn transform_eval_expressions_root(
    value: &Value,
    context: &mut ExprEvalContext,
) -> Result<Value> {
    context.namespace = None;

    if let Value::Mapping(map) = value {
        if let Some(namespace) = extract_namespace_from_map(&map, context)? {
            context.namespace = Some(namespace);
        } else {
            context.namespace = Some(DEFAULT_NAMESPACE.to_string());
        }
    }

    transform_eval_expressions(value, context)
}

fn extract_namespace_from_map(map: &Mapping, context: &ExprEvalContext) -> Result<Option<String>> {
    if map.len() != 1 {
        return Ok(None);
    }

    let single_key = map.keys().next().unwrap();
    let Some(resource) = map.get(single_key).unwrap().as_mapping().cloned() else {
        return Ok(None);
    };

    let Some(Value::String(namespace)) = resource.get("namespace") else {
        return Ok(None);
    };

    let result = parse_and_eval_expr(namespace, context);
    let namespace = match result {
        Ok(Some(Value::String(namespace))) => namespace,
        Ok(None) => namespace.clone(),
        _ => bail!("Failed to evaluate namespace exp {:?}", result),
    };

    Ok(Some(namespace))
}

pub fn transform_eval_expressions(value: &Value, context: &ExprEvalContext) -> Result<Value> {
    if let Some(str) = value.as_str() {
        let new_value = parse_and_eval_expr(str, context)?;
        return Ok(new_value.unwrap_or(value.clone()));
    }

    if let Some(map) = value.as_mapping() {
        let mut new_map = Mapping::new();
        for (key, value) in map {
            new_map.insert(key.clone(), transform_eval_expressions(value, context)?);
        }
        Ok(Value::Mapping(new_map))
    } else if let Some(seq) = value.as_sequence() {
        let mut new_seq = Sequence::new();
        for value in seq {
            new_seq.push(transform_eval_expressions(value, context)?);
        }
        Ok(Value::Sequence(new_seq))
    } else {
        Ok(value.clone())
    }
}

fn parse_and_eval_expr(expr: &str, context: &ExprEvalContext) -> Result<Option<Value>> {
    // either
    // 1. it starts with ${{ and ends with }} => we eval the expression and return the result as a value
    // 2. or it contains ${{ and }} => we eval the expression/s, convert the result to a string and replace in the original string
    // 3. or is just a regular string => we return the original string

    let expr = expr.trim();

    let expr_start_marker_count = expr.matches("${{").count();
    let expr_end_marker_count = expr.matches("}}").count();

    if expr_start_marker_count == 0 && expr_end_marker_count == 0 {
        return Ok(None);
    }

    if expr.starts_with("${{")
        && expr.ends_with("}}")
        && expr_start_marker_count == 1
        && expr_end_marker_count == 1
    {
        let expr = expr
            .trim_start_matches("${{")
            .trim_end_matches("}}")
            .trim()
            .to_string();

        return eval_expr(&expr, context).map(|v| Some(v));
    }

    // loop should be find, split, eval, replace, repeat\
    let mut output = expr.to_string();
    loop {
        let start = output.find("${{").unwrap_or(0);
        let end = output.find("}}").unwrap_or(0);

        if start == 0 && end == 0 {
            break;
        }

        let expr = output[start + 3..end - 1].trim();

        if expr.is_empty() {
            break;
        }

        let value = eval_expr(&expr, context)?;
        let value_str = match value {
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => n.to_string(),
            Value::String(s) => s.to_string(),
            Value::Null => "null".to_string(),
            _ => bail!(
                "Invalid value '{:?}' returned by expression '{}'",
                value,
                expr
            ),
        };
        output = output[..start].to_string() + &value_str + &output[end + 2..];
    }

    return Ok(Some(Value::String(output)));
}
