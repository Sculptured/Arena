//! How a tool call fails: a message for the model, or a database error.

pub(super) enum Fail {
    Tool(String),
    Db(anyhow::Error),
}

impl From<tokio_postgres::Error> for Fail {
    fn from(e: tokio_postgres::Error) -> Self {
        Fail::Db(e.into())
    }
}

pub(super) fn bad<T>(msg: impl Into<String>) -> Result<T, Fail> {
    Err(Fail::Tool(msg.into()))
}

pub(super) type R<T> = Result<T, Fail>;
