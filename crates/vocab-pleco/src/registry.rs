use std::collections::BTreeMap;

use crate::codec::{CodecIdent, PlecoCodec, Utf8TextV1};

pub struct CodecRegistry {
    codecs: BTreeMap<String, Box<dyn PlecoCodec>>,
}

impl CodecRegistry {
    /// The built-in, documented set of format variants. Rejected or unlisted
    /// variants are absent here on purpose — callers must fail loudly rather than
    /// guess at an unknown Pleco grammar.
    #[must_use]
    pub fn builtin() -> Self {
        let mut registry = Self {
            codecs: BTreeMap::new(),
        };
        registry.register(Utf8TextV1);
        registry
    }

    pub fn register(&mut self, codec: impl PlecoCodec + 'static) {
        let ident = codec.ident();
        self.codecs.insert(ident.key(), Box::new(codec));
    }

    pub fn get(&self, ident: &CodecIdent) -> Option<&dyn PlecoCodec> {
        self.codecs.get(&ident.key()).map(Box::as_ref)
    }

    /// Looks a codec up by its `format/variant` key string (e.g.
    /// `pleco-utf8-text/v1`). Returns `None` for unknown or malformed keys so
    /// callers fail loudly instead of guessing.
    #[must_use]
    pub fn get_by_key(&self, key: &str) -> Option<&dyn PlecoCodec> {
        self.codecs.get(key).map(Box::as_ref)
    }

    /// All supported variants, sorted for deterministic presentation.
    #[must_use]
    pub fn supported(&self) -> Vec<CodecIdent> {
        self.codecs.values().map(|codec| codec.ident()).collect()
    }
}
