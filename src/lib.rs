extern crate grep;
use grep::regex::RegexMatcher;
use grep::searcher::sinks::UTF8;
use grep::searcher::{Searcher, Sink, SinkError, SinkMatch};

use std::os::raw::c_char;
use std::os::raw::c_int;

use std::fmt;
use std::fs::File;
use std::io;

use std::ffi::*;

fn main() {
    search_for_bees(io::stdout()).unwrap();
}

fn search_for_bees<W: io::Write>(mut out: W) -> Result<(), io::Error> {
    let sink = UTF8(|line: u64, text: &str| {
        writeln!(out, "Match at line {}: {}", line, text)?;
        Ok(true)
    });

    // finds every "bee" in Bee Movie
    let matcher = RegexMatcher::new("[Bb]ee").expect("Could not form bee-matching RegexMatcher");

    let subject = File::open("bee_movie.txt")
        .expect("Could not open the entire script of Bee Movie in bee_movie.txt");

    Searcher::new().search_file(&matcher, &subject, sink)?;

    Ok(())
}

// For use returning back through the FFI.
// Note that the bytes inside are NOT nul-terminated!
#[repr(C)]
struct SearchResult {
    line_number: c_int,
    bytes: *const u8,
    num_bytes: c_int,
}

#[repr(C)]
#[derive(Debug, Eq, PartialEq, Clone, Copy)]
enum SearchStatusCode {
    Success = 0,
    // Equivalents to IllegalArgumentExceptions:
    MissingFilename = 1,
    MissingSearchText = 2,
    MissingCallback = 3,
    // Failure from inside ripgrep:
    ErrorBadPattern = 11,
    ErrorCouldNotOpenFile = 12,
    ErrorFromRipgrep = 13,
    // Failure from inside the callback:
    ErrorFromCallback = 21,
}

// indicates Success on true, Failure on false
type SearchResultCallback = extern "C" fn(SearchResult) -> bool;

struct SearchResultCallbackSink(SearchResultCallback);

struct CallbackError {
    error_message: String,
}

impl SinkError for CallbackError {
    fn error_message<T: fmt::Display>(message: T) -> Self {
        Self {
            error_message: format!("{}", message),
        }
    }
}

impl Sink for SearchResultCallbackSink {
    type Error = CallbackError;

    fn matched(
        &mut self,
        _searcher: &Searcher,
        matched: &SinkMatch,
    ) -> Result<bool, CallbackError> {
        let result = SearchResult {
            // -1 is a common value to use in Java when an int value is not found
            line_number: matched.line_number().map(|n| n as c_int).unwrap_or(-1),
            // lifetime should be good because the callback will finish before the buffer is modified.
            bytes: matched.bytes().as_ptr(),
            num_bytes: matched.bytes().len() as c_int,
        };
        let succeeded: bool = (self.0)(result);
        if succeeded {
            Ok(true) // callback done, keep searching
        } else {
            Err(CallbackError::error_message(
                "Callback completed but indicated an error",
            ))
        }
    }
}

#[no_mangle]
extern "C" fn search_file(
    // every Java type is nullable, represented here as an Option<*type>
    filename: Option<*const c_char>,
    search_text: Option<*const c_char>,
    result_callback: Option<SearchResultCallback>,
) -> SearchStatusCode {
    use SearchStatusCode::*;

    // Java owns the string, so we view the text as a &CStr reference rather than an owned CString
    let filename: &CStr = match filename {
        None => return MissingFilename,
        Some(filename_ptr) => unsafe { CStr::from_ptr(filename_ptr) },
    };

    let filename: &str = match filename.to_str() {
        Ok(filename) => filename,
        Err(_) => return ErrorCouldNotOpenFile,
    };

    let file: File = match File::open(filename) {
        Ok(file) => file,
        Err(_) => return ErrorCouldNotOpenFile,
    };

    // Java owns the string, so we view the text as a &CStr reference rather than an owned CString
    let search_text: &CStr = match search_text {
        None => return MissingSearchText,
        Some(search_text_ptr) => unsafe { CStr::from_ptr(search_text_ptr) },
    };

    let search_text: &str = match search_text.to_str() {
        Ok(search_text) => search_text,
        Err(_) => return ErrorBadPattern,
    };

    let matcher = match RegexMatcher::new(search_text) {
        Ok(regex) => regex,
        Err(_) => return ErrorBadPattern,
    };

    let result_callback: SearchResultCallback = match result_callback {
        Some(callback) => callback,
        None => return MissingCallback,
    };

    let sink = SearchResultCallbackSink(result_callback);

    match Searcher::new().search_file(&matcher, &file, sink) {
        Ok(_) => Success,
        Err(_) => ErrorFromCallback,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_for_bees_without_error() {
        let filename = CString::new("bee_movie.txt")
            .expect("Could not represent \"bee_movie.txt\" as a CString");
        let search_pattern =
            CString::new("[Bb]ee").expect("Could not represent \"[Bb]ee\" as a CString");
        let callback = always_succeeding_callback;

        let result_code = search_file(
            Some(filename.as_ptr()),
            Some(search_pattern.as_ptr()),
            Some(callback),
        );

        assert_eq!(SearchStatusCode::Success, result_code,
        	"When the callback returns true to indicate success, the extern search_file function should always return {:?}", SearchStatusCode::Success);
    }

    #[test]
    fn test_search_for_bees_returns_callback_error_code_when_callback_returns_false() {
        let filename = CString::new("bee_movie.txt")
            .expect("Could not represent \"bee_movie.txt\" as a CString");
        let search_pattern =
            CString::new("[Bb]ee").expect("Could not represent \"[Bb]ee\" as a CString");
        let callback = always_failing_callback;

        let result_code = search_file(
            Some(filename.as_ptr()),
            Some(search_pattern.as_ptr()),
            Some(callback),
        );

        assert_eq!(SearchStatusCode::ErrorFromCallback, result_code,
        	"When the callback returns false to indicate an error, the extern search_file function should always return {:?}", SearchStatusCode::ErrorFromCallback);
    }

    #[test]
    fn test_search_for_bees_returns_could_not_open_file_error_code_when_searching_missing_file() {
        let filename = CString::new("non_existant_file.txt")
            .expect("Could not represent \"non_existant_file.txt\" as a CString");
        let search_pattern =
            CString::new("[Bb]ee").expect("Could not represent \"[Bb]ee\" as a CString");
        let callback = always_failing_callback;

        let result_code = search_file(
            Some(filename.as_ptr()),
            Some(search_pattern.as_ptr()),
            Some(callback),
        );

        assert_eq!(SearchStatusCode::ErrorCouldNotOpenFile, result_code,
        	"When passing the name of a file that does not exist, the extern search_file function should always return {:?}", SearchStatusCode::ErrorCouldNotOpenFile);
    }

    extern "C" fn always_succeeding_callback(_: SearchResult) -> bool {
        true
    }

    extern "C" fn always_failing_callback(_: SearchResult) -> bool {
        false
    }
}
