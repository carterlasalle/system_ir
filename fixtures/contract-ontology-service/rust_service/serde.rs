//! Contract-ontology rust half: Serialize/Deserialize pairs around types —
//! one via derive, one via explicit `impl` blocks (serialization contracts).

use serde::{Deserialize, Serialize};

/// Derive-based serializer/deserializer pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub retries: u32,
}

/// Manual serializer/deserializer pair around the type.
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Serialize for Point {
    fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
        unimplemented!()
    }
}

impl Deserialize for Point {
    fn deserialize<D>(_deserializer: D) -> Result<Point, D::Error> {
        unimplemented!()
    }
}
