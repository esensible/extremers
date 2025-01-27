use ::serde::Serialize;

#[derive(Serialize, Copy, Clone, PartialEq, Default)]
pub struct Location {
    pub lat: f64,
    pub lon: f64,
}
