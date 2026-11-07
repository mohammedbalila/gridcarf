//! Serde helpers for maps keyed by `CellRef`. JSON object keys must be strings, so the keys are
//! written as `A1`-style text.

use std::collections::BTreeMap;

use gridcraft_core::CellRef;
use serde::de::Error;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub fn serialize<S: Serializer, V: Serialize>(map: &BTreeMap<CellRef, V>, s: S) -> Result<S::Ok, S::Error> {
    s.collect_map(map.iter().map(|(k, v)| (k.a1(), v)))
}

pub fn deserialize<'de, D: Deserializer<'de>, V: Deserialize<'de>>(d: D) -> Result<BTreeMap<CellRef, V>, D::Error> {
    let raw = BTreeMap::<String, V>::deserialize(d)?;
    let mut out = BTreeMap::new();
    for (k, v) in raw {
        let cell = CellRef::parse(&k).ok_or_else(|| D::Error::custom(format!("invalid cell reference `{k}`")))?;
        out.insert(cell, v);
    }
    Ok(out)
}

/// The same A1-keyed wire shape for maps shared between sheet snapshots.
pub mod shared {
    use std::sync::Arc;

    use super::{BTreeMap, CellRef, Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer, V: Serialize>(map: &Arc<BTreeMap<CellRef, V>>, s: S) -> Result<S::Ok, S::Error> {
        super::serialize(map, s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>, V: Deserialize<'de>>(d: D) -> Result<Arc<BTreeMap<CellRef, V>>, D::Error> {
        super::deserialize(d).map(Arc::new)
    }
}

#[cfg(test)]
mod tests {
    use crate::{Comment, Hyperlink, Sheet};
    use gridcraft_core::CellRef;

    #[test]
    fn comments_and_hyperlinks_survive_json() -> Result<(), Box<dyn std::error::Error>> {
        let mut s = Sheet::new("S");
        let a1 = CellRef::new(0, 0);
        let c3 = CellRef::new(2, 2);
        s.comments_mut().insert(
            a1,
            Comment {
                author: "Example".into(),
                text: "note".into(),
                replies: vec![("B".into(), "r".into())],
                threaded: true,
                resolved: false,
                visible: false,
                dates: vec![],
                note_box: None,
            },
        );
        s.hyperlinks.insert(c3, Hyperlink { target: "Sheet1!A1".into(), tooltip: Some("jump".into()), external: false });
        let json = serde_json::to_string(&s)?;
        let wire: serde_json::Value = serde_json::from_str(&json)?;
        assert_eq!(wire["comments"]["A1"]["text"], "note");
        assert_eq!(wire["hyperlinks"]["C3"]["target"], "Sheet1!A1");
        assert_eq!(wire["comments"].as_object().unwrap().len(), 1);
        // Historical JSON uses plain A1-keyed objects, without a sharing wrapper.
        let mut old = serde_json::to_value(Sheet::new("Old"))?;
        old["comments"] =
            serde_json::json!({"B2": {"author": "Old", "text": "legacy", "threaded": false, "resolved": false, "visible": false, "replies": []}});
        let old: Sheet = serde_json::from_value(old)?;
        assert_eq!(old.comments[&CellRef::new(1, 1)].text, "legacy");
        let back: Sheet = serde_json::from_str(&json)?;
        assert_eq!(back.comments, s.comments);
        assert_eq!(back.hyperlinks, s.hyperlinks);
        Ok(())
    }
}
