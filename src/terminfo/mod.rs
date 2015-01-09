// Copyright 2012-2014 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Terminfo database interface.

use std::collections::HashMap;
use std::io::IoResult;
use std::os;

use Attr;
use color;
use Terminal;
use UnwrappableTerminal;
use self::searcher::open;
use self::parser::compiled::{parse, msys_terminfo};
use self::parm::{expand, Variables, Param};


/// A parsed terminfo database entry.
#[derive(Show)]
pub struct TermInfo {
    /// Names for the terminal
    pub names: Vec<String> ,
    /// Map of capability name to boolean value
    pub bools: HashMap<String, bool>,
    /// Map of capability name to numeric value
    pub numbers: HashMap<String, u16>,
    /// Map of capability name to raw (unexpanded) string
    pub strings: HashMap<String, Vec<u8> >
}

pub mod searcher;

/// TermInfo format parsing.
pub mod parser {
    //! ncurses-compatible compiled terminfo format parsing (term(5))
    pub mod compiled;
}
pub mod parm;


fn cap_for_attr(attr: Attr) -> &'static str {
    match attr {
        Attr::Bold               => "bold",
        Attr::Dim                => "dim",
        Attr::Italic(true)       => "sitm",
        Attr::Italic(false)      => "ritm",
        Attr::Underline(true)    => "smul",
        Attr::Underline(false)   => "rmul",
        Attr::Blink              => "blink",
        Attr::Standout(true)     => "smso",
        Attr::Standout(false)    => "rmso",
        Attr::Reverse            => "rev",
        Attr::Secure             => "invis",
        Attr::ForegroundColor(_) => "setaf",
        Attr::BackgroundColor(_) => "setab"
    }
}

/// A Terminal that knows how many colors it supports, with a reference to its
/// parsed Terminfo database record.
pub struct TerminfoTerminal<T> {
    num_colors: u16,
    out: T,
    ti: TermInfo,
}

impl<T: Writer+Send> Terminal<T> for TerminfoTerminal<T> {
    fn fg(&mut self, color: color::Color) -> IoResult<bool> {
        let color = self.dim_if_necessary(color);
        if self.num_colors > color {
            return self.apply_cap("setaf", &[Param::Number(color as int)]);
        }
        Ok(false)
    }

    fn bg(&mut self, color: color::Color) -> IoResult<bool> {
        let color = self.dim_if_necessary(color);
        if self.num_colors > color {
            return self.apply_cap("setab", &[Param::Number(color as int)]);
        }
        Ok(false)
    }

    fn attr(&mut self, attr: Attr) -> IoResult<bool> {
        match attr {
            Attr::ForegroundColor(c) => self.fg(c),
            Attr::BackgroundColor(c) => self.bg(c),
            _ => self.apply_cap(cap_for_attr(attr), &[]),
        }
    }

    fn supports_attr(&self, attr: Attr) -> bool {
        match attr {
            Attr::ForegroundColor(_) | Attr::BackgroundColor(_) => {
                self.num_colors > 0
            }
            _ => {
                let cap = cap_for_attr(attr);
                self.ti.strings.get(cap).is_some()
            }
        }
    }

    fn reset(&mut self) -> IoResult<bool> {
        // are there any terminals that have color/attrs and not sgr0?
        // Try falling back to sgr, then op
        let cmd = match [
            "sg0", "sgr", "op"
        ].iter().filter_map(|cap| {
            self.ti.strings.get(*cap)
        }).next() {
            Some(op) => match expand(&op[], &[], &mut Variables::new()) {
                Ok(cmd) => cmd,
                Err(_) => return Ok(false),
            },
            None => return Ok(false),
        };

        self.out.write(&cmd[]).map(|_|true)
    }

    fn get_ref<'a>(&'a self) -> &'a T { &self.out }

    fn get_mut<'a>(&'a mut self) -> &'a mut T { &mut self.out }
}

impl<T: Writer+Send> UnwrappableTerminal<T> for TerminfoTerminal<T> {
    fn unwrap(self) -> T { self.out }
}

impl<T: Writer+Send> TerminfoTerminal<T> {
    /// Returns `None` whenever the terminal cannot be created for some
    /// reason.
    pub fn new(out: T) -> Option<TerminfoTerminal<T>> {
        let term = match os::getenv("TERM") {
            Some(t) => t,
            None => {
                debug!("TERM environment variable not defined");
                return None;
            }
        };

        let entry = open(&term[]);
        if entry.is_err() {
            if os::getenv("MSYSCON").map_or(false, |s| {
                    "mintty.exe" == s
                }) {
                // msys terminal
                return Some(TerminfoTerminal {
                    out: out,
                    ti: msys_terminfo(),
                    num_colors: 8
                });
            }
            debug!("error finding terminfo entry: {}", entry.err().unwrap());
            return None;
        }

        let mut file = entry.unwrap();
        let ti = parse(&mut file, false);
        if ti.is_err() {
            debug!("error parsing terminfo entry: {}", ti.unwrap_err());
            return None;
        }

        let inf = ti.unwrap();
        let nc = if inf.strings.get("setaf").is_some()
                 && inf.strings.get("setab").is_some() {
                     inf.numbers.get("colors").map_or(0, |&n| n)
                 } else { 0 };

        return Some(TerminfoTerminal {
            out: out,
            ti: inf,
            num_colors: nc
        });
    }

    fn dim_if_necessary(&self, color: color::Color) -> color::Color {
        if color >= self.num_colors && color >= 8 && color < 16 {
            color-8
        } else { color }
    }

    fn apply_cap(&mut self, cmd: &str, params: &[Param]) -> IoResult<bool> {
        if let Some(cmd) = self.ti.strings.get(cmd) {
            if let Ok(s) = expand(cmd.as_slice(), params, &mut Variables::new()) {
                try!(self.out.write(s.as_slice()));
                return Ok(true)
            }
        }
        Ok(false)
    }
}


impl<T: Writer> Writer for TerminfoTerminal<T> {
    fn write(&mut self, buf: &[u8]) -> IoResult<()> {
        self.out.write(buf)
    }

    fn flush(&mut self) -> IoResult<()> {
        self.out.flush()
    }
}
