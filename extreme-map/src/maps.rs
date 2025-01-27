pub struct MapDisplay {
    map_bounds: MapBounds,
    current_position: Position,
}

pub struct MapBounds {
    top_left: Position,
    bottom_right: Position,
    image_width: u32,
    image_height: u32,
}

pub struct Position {
    lat: f64,
    lon: f64,
}

impl MapDisplay {
    pub fn new(map_bounds: MapBounds) -> Self {
        Self {
            map_bounds,
            current_position: Position { lat: 0.0, lon: 0.0 },
        }
    }

    pub fn update_position(&mut self, lat: f64, lon: f64) {
        self.current_position = Position { lat, lon };
    }
}
