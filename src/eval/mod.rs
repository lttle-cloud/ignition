pub mod ctx;
pub mod std_lib;

pub trait CelCtxExt {
    fn add_stdlib_functions(&mut self);
}

#[cfg(feature = "daemon")]
pub use crate::cel_functions::CelResourceExt;

impl CelCtxExt for cel::Context<'_> {
    fn add_stdlib_functions(&mut self) {
        // custom string functions
        self.add_function("last", std_lib::str::last);
        self.add_function("slugify", std_lib::str::slugify);
        self.add_function("toSlug", std_lib::str::to_slug);

        // CEL extended string functions
        self.add_function("charAt", std_lib::str::char_at);
        self.add_function("indexOf", std_lib::str::index_of);
        self.add_function("join", std_lib::str::join_list);
        self.add_function("lastIndexOf", std_lib::str::last_index_of);
        self.add_function("lowerAscii", std_lib::str::lower_ascii);
        self.add_function("quote", std_lib::str::quote);
        self.add_function("replace", std_lib::str::replace);
        self.add_function("split", std_lib::str::split_string);
        self.add_function("substring", std_lib::str::substring);
        self.add_function("trim", std_lib::str::trim);
        self.add_function("upperAscii", std_lib::str::upper_ascii);
        self.add_function("reverse", std_lib::str::reverse);
    }
}
