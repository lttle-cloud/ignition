use anyhow::{Result, bail};
use serde_yaml::Value;

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
