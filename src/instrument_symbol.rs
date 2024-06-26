use std::fmt::Display;
use std::ops::Deref;
use compact_str::CompactString;
use rust_extensions::sorted_vec::EntityWithKey;

#[derive(Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Debug)]
pub struct InstrumentSymbol(pub CompactString);

impl Deref for InstrumentSymbol {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0.as_str()
    }
}

impl From<&str> for InstrumentSymbol {
    fn from(value: &str) -> Self {
        InstrumentSymbol(value.into())
    }
}

impl From<String> for InstrumentSymbol {
    fn from(value: String) -> Self {
        InstrumentSymbol(value.into())
    }
}

impl From<CompactString> for InstrumentSymbol {
    fn from(value: CompactString) -> Self {
        InstrumentSymbol(value)
    }
}

impl From<&CompactString> for InstrumentSymbol {
    fn from(value: &CompactString) -> Self {
        InstrumentSymbol(value.to_owned())
    }
}

impl From<&String> for InstrumentSymbol {
    fn from(value: &String) -> Self {
        InstrumentSymbol(value.into())
    }
}


impl Display for InstrumentSymbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.to_string())
    }
}

impl EntityWithKey<InstrumentSymbol> for InstrumentSymbol {
    fn get_key(&self) -> &InstrumentSymbol {
        self
    }
}