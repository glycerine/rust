// Copyright 2012-2013 The Rust Project Developers. See the
// COPYRIGHT file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use ast;
use codemap;
use codemap::{Pos, Span};
use codemap::{ExpnInfo, NameAndSpan};
use ext::base::*;
use ext::base;
use ext::build::AstBuilder;
use parse;
use parse::token::{get_ident_interner};
use print::pprust;

use std::io;
use std::result;

// These macros all relate to the file system; they either return
// the column/row/filename of the expression, or they include
// a given file into the current one.

/* line!(): expands to the current line number */
pub fn expand_line(cx: @ExtCtxt, sp: Span, tts: &[ast::token_tree])
    -> base::MacResult {
    base::check_zero_tts(cx, sp, tts, "line!");

    let topmost = topmost_expn_info(cx.backtrace().unwrap());
    let loc = cx.codemap().lookup_char_pos(topmost.call_site.lo);

    base::MRExpr(cx.expr_uint(topmost.call_site, loc.line))
}

/* col!(): expands to the current column number */
pub fn expand_col(cx: @ExtCtxt, sp: Span, tts: &[ast::token_tree])
    -> base::MacResult {
    base::check_zero_tts(cx, sp, tts, "col!");

    let topmost = topmost_expn_info(cx.backtrace().unwrap());
    let loc = cx.codemap().lookup_char_pos(topmost.call_site.lo);
    base::MRExpr(cx.expr_uint(topmost.call_site, loc.col.to_uint()))
}

/* file!(): expands to the current filename */
/* The filemap (`loc.file`) contains a bunch more information we could spit
 * out if we wanted. */
pub fn expand_file(cx: @ExtCtxt, sp: Span, tts: &[ast::token_tree])
    -> base::MacResult {
    base::check_zero_tts(cx, sp, tts, "file!");

    let topmost = topmost_expn_info(cx.backtrace().unwrap());
    let loc = cx.codemap().lookup_char_pos(topmost.call_site.lo);
    let filename = loc.file.name;
    base::MRExpr(cx.expr_str(topmost.call_site, filename))
}

/** funcpathfile!(): expands to the current fully-qualified-function name 
    plus file path:line:col for lambda disambiguation
    See also funcpath!() and func!().
    For performance reasons this needs to end up as a static 
    string and not a format!-ed heap-allocation.
*/
pub fn expand_funcpathfile(cx: @ExtCtxt, sp: Span, tts: &[ast::token_tree])
    -> base::MacResult {
    base::check_zero_tts(cx, sp, tts, "funcpathfile!");
    let depth = cx.func_depth();
    if depth == 0 {
        cx.span_err(sp, 
                      format!("funcpathfile!() called when not inside a function"));
    }

    let nearest = cx.backtrace().unwrap();
    let loc = cx.codemap().lookup_char_pos(nearest.call_site.lo);
    let filename = loc.file.name;
    let linestr : ~str = loc.line.to_str();
    let colstr  : ~str = loc.col.to_str();

    let res = filename + ":" + linestr + ":" + colstr + "|" + cx.func_path();

    base::MRExpr(cx.expr_str(sp, res.to_managed()))
}

/** function_path!(): expands to the current fully-qualified-function name.
   Anonymous functions or lambdas will be ambiguously all named 'lambda'. 
   Use funcpathfile!() if you require disambiguation (file:line:col details)
   of lambdas or wish to have exact locations at your fingertips. See also
   func!() for shortest-possible (basename) function name.
   For performance reasons this needs to end up as a static 
   string and not a format!-ed heap-allocation.
*/
pub fn expand_function_path(cx: @ExtCtxt, sp: Span, tts: &[ast::token_tree])
    -> base::MacResult {
    base::check_zero_tts(cx, sp, tts, "function_path!");
    let depth = cx.func_depth();
    if depth == 0 {
        cx.span_err(sp, 
                      format!("function_path!() called when not inside a function"));
    }
    base::MRExpr(cx.expr_str(sp, cx.func_path().to_managed()))
}

/** func!(): expands to the shortname or basename of the current
   function name. See also funcpath!() and funcpathfile!(). */
pub fn expand_function(cx: @ExtCtxt, sp: Span, tts: &[ast::token_tree])
    -> base::MacResult {
    base::check_zero_tts(cx, sp, tts, "function!");
    let depth = cx.func_depth();
    if depth == 0 {
        cx.span_err(sp, 
                      format!("function!() called when not inside a function"));
    }
    match cx.func_path_last() {
        None => {
            // should never get here with the depth check above.
            cx.span_fatal(sp, format!("function!() called when not inside a function"))
        }
        Some(func_shortname) => base::MRExpr(cx.expr_str(sp, cx.str_of(func_shortname)))
    }
}


pub fn expand_stringify(cx: @ExtCtxt, sp: Span, tts: &[ast::token_tree])
    -> base::MacResult {
    let s = pprust::tts_to_str(tts, get_ident_interner());
    base::MRExpr(cx.expr_str(sp, s.to_managed()))
}

pub fn expand_mod(cx: @ExtCtxt, sp: Span, tts: &[ast::token_tree])
    -> base::MacResult {
    base::check_zero_tts(cx, sp, tts, "module_path!");
    base::MRExpr(cx.expr_str(sp,
                             cx.mod_path().map(|x| cx.str_of(*x)).connect("::").to_managed()))
}

// include! : parse the given file as an expr
// This is generally a bad idea because it's going to behave
// unhygienically.
pub fn expand_include(cx: @ExtCtxt, sp: Span, tts: &[ast::token_tree])
    -> base::MacResult {
    let file = get_single_str_from_tts(cx, sp, tts, "include!");
    let p = parse::new_sub_parser_from_file(
        cx.parse_sess(), cx.cfg(),
        &res_rel_file(cx, sp, &Path(file)), sp);
    base::MRExpr(p.parse_expr())
}

// include_str! : read the given file, insert it as a literal string expr
pub fn expand_include_str(cx: @ExtCtxt, sp: Span, tts: &[ast::token_tree])
    -> base::MacResult {
    let file = get_single_str_from_tts(cx, sp, tts, "include_str!");
    let res = io::read_whole_file_str(&res_rel_file(cx, sp, &Path(file)));
    match res {
      result::Ok(res) => {
          base::MRExpr(cx.expr_str(sp, res.to_managed()))
      }
      result::Err(e) => {
        cx.span_fatal(sp, e);
      }
    }
}

pub fn expand_include_bin(cx: @ExtCtxt, sp: Span, tts: &[ast::token_tree])
    -> base::MacResult {
    let file = get_single_str_from_tts(cx, sp, tts, "include_bin!");
    match io::read_whole_file(&res_rel_file(cx, sp, &Path(file))) {
      result::Ok(src) => {
        let u8_exprs: ~[@ast::Expr] = src.iter().map(|char| cx.expr_u8(sp, *char)).collect();
        base::MRExpr(cx.expr_vec(sp, u8_exprs))
      }
      result::Err(ref e) => {
        cx.parse_sess().span_diagnostic.handler().fatal((*e))
      }
    }
}

// recur along an ExpnInfo chain to find the original expression
fn topmost_expn_info(expn_info: @codemap::ExpnInfo) -> @codemap::ExpnInfo {
    match *expn_info {
        ExpnInfo { call_site: ref call_site, _ } => {
            match call_site.expn_info {
                Some(next_expn_info) => {
                    match *next_expn_info {
                        ExpnInfo {
                            callee: NameAndSpan { name: ref name, _ },
                            _
                        } => {
                            // Don't recurse into file using "include!"
                            if "include" == *name  {
                                expn_info
                            } else {
                                topmost_expn_info(next_expn_info)
                            }
                        }
                    }
                },
                None => expn_info
            }
        }
    }
}

// resolve a file-system path to an absolute file-system path (if it
// isn't already)
fn res_rel_file(cx: @ExtCtxt, sp: codemap::Span, arg: &Path) -> Path {
    // NB: relative paths are resolved relative to the compilation unit
    if !arg.is_absolute {
        let cu = Path(cx.codemap().span_to_filename(sp));
        cu.dir_path().push_many(arg.components)
    } else {
        (*arg).clone()
    }
}
