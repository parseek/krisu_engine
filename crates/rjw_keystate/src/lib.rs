#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KeyState(u8);

pub const KEY_STATE_MASK_PRESSED: u8 = 0b0001;
pub const KEY_STATE_MASK_EDGE: u8 = 0b0010;
pub const KEY_STATE_MASK_TRUE_EDGE: u8 = 0b0110;
pub const KEY_STATE_MASK_TRUE_EDGE_BIT: u8 = 0b0100;
pub const KEY_STATE_MASK_SUDDEN_UP_BIT: u8 = 0b1000;

#[allow(unused)]
pub const KEY_STATE_RELEASED: KeyState = KeyState(0);
#[allow(unused)]
pub const KEY_STATE_PRESSING: KeyState = KeyState(KEY_STATE_MASK_PRESSED);

/// Up edge provided by OS, but it may not be a "true" edge.
#[allow(unused)]
pub const KEY_STATE_UP_EDGE: KeyState = KeyState(KEY_STATE_MASK_EDGE);

/// Some OSes may not provide true edge detection, so we can use this constant to represent a up edge that is also a true edge.
#[allow(unused)]
pub const KEY_STATE_UP_TRUE_EDGE: KeyState =
    KeyState(KEY_STATE_MASK_EDGE | KEY_STATE_MASK_TRUE_EDGE);

/// Down edge provided by OS, but it may not be a "true" edge.
/// Used for typing etc.
#[allow(unused)]
pub const KEY_STATE_DOWN_EDGE: KeyState = KeyState(KEY_STATE_MASK_PRESSED | KEY_STATE_MASK_EDGE);

/// Some OSes may not provide true edge detection, so we can use this constant to represent a down edge that is also a true edge.
#[allow(unused)]
pub const KEY_STATE_DOWN_TRUE_EDGE: KeyState =
    KeyState(KEY_STATE_MASK_PRESSED | KEY_STATE_MASK_EDGE | KEY_STATE_MASK_TRUE_EDGE);

impl Default for KeyState {
    fn default() -> Self {
        KEY_STATE_RELEASED
    }
}

// Prints the state of the key, for debugging purposes.
impl std::fmt::Display for KeyState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "KeyState {{ pressed: {}, edge: {}, true_edge: {} }}",
            self.pressed(),
            self.edge(),
            self.true_edge()
        )
    }
}

impl KeyState {
    #[allow(unused)]
    pub fn pressed(&self) -> bool {
        (self.0 & KEY_STATE_MASK_PRESSED) != 0
    }
    #[allow(unused)]
    pub fn released(&self) -> bool {
        !self.pressed()
    }
    #[allow(unused)]
    pub fn edge(&self) -> bool {
        (self.0 & KEY_STATE_MASK_EDGE) != 0
    }
    #[allow(unused)]
    pub fn true_edge(&self) -> bool {
        (self.0 & KEY_STATE_MASK_TRUE_EDGE_BIT) != 0
    }
    #[allow(unused)]
    pub fn up_edge(&self) -> bool {
        self.edge() && self.released()
    }
    #[allow(unused)]
    pub fn down_edge(&self) -> bool {
        self.edge() && self.pressed()
    }
    #[allow(unused)]
    pub fn up_true_edge(&self) -> bool {
        self.true_edge() && self.released()
    }
    #[allow(unused)]
    pub fn down_true_edge(&self) -> bool {
        self.true_edge() && self.pressed()
    }
    #[allow(unused)]
    pub fn sudden_up(&self) -> bool {
        (self.0 & KEY_STATE_MASK_SUDDEN_UP_BIT) != 0
    }
    pub fn off_edge(self) -> KeyState {
        KeyState(self.0 & !(KEY_STATE_MASK_EDGE | KEY_STATE_MASK_TRUE_EDGE))
    }
    pub fn set_sudden_up(self) -> KeyState {
        KeyState(self.0 | KEY_STATE_MASK_SUDDEN_UP_BIT)
    }
}
