use std::collections::HashSet;
use std::cell::UnsafeCell;

#[derive(Debug, Default)]
pub struct Interner {
    set: UnsafeCell<HashSet<String>>
}

impl Interner {
    pub fn intern(&self, s: &str) -> &str {
        unsafe {
            if let Some(string) = (*self.set.get()).get(s) {
                string
            } else {
                let string = s.to_owned();
                let ret = &*(&string as &str as *const str);
                (*self.set.get()).insert(string);
                ret
            }
        }
    }
}
