use serde::Serialize;

const MAX_INPUT_BYTES: usize = 8 * 1024;

/// Stable, deliberately small snapshot passed to an external status formatter.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StatusLineCommandInput {
    pub(crate) cwd: String,
    pub(crate) model: String,
    pub(crate) status: String,
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) context_remaining_percent: Option<i64>,
}

impl StatusLineCommandInput {
    pub(crate) fn to_json_line(&self) -> Result<Vec<u8>, &'static str> {
        let mut bytes =
            serde_json::to_vec(self).map_err(|_| "could not serialize formatter input")?;
        if bytes.len() >= MAX_INPUT_BYTES {
            return Err("formatter input exceeded 8192 bytes");
        }
        bytes.push(b'\n');
        Ok(bytes)
    }
}

#[cfg(test)]
#[path = "wire_tests.rs"]
mod tests;
