pub struct Store;

impl Store {
    pub fn connect(_url: &str) -> Store {
        Store::new()
    }

    pub fn new() -> Store {
        Store
    }
}

pub struct Cache;

impl Cache {
    pub fn new() -> Cache {
        Cache
    }
}
