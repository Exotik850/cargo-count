use std::io::{self, Read, Write};
use std::fs::File;
use std::path::{Path, PathBuf};

use tabwriter::TabWriter;

use comment::Comment;
use config::{Config, Utf8Rule};
use count::Count;
use error::{CliError, CliResult};
use fsutil;
use fmt::{self, Format};
use language::Language;

pub struct Counts<'c> {
    cfg: &'c Config<'c>,
    counts: Vec<Count>,
    tot: usize,
    tot_lines: u64,
    tot_comments: u64,
    tot_blanks: u64,
    tot_code: u64,
    tot_usafe: u64
}

impl<'c> Counts<'c> {
    pub fn new(cfg: &'c Config) -> Self {
        Counts {
            cfg: cfg,
            counts: vec![],
            tot: 0,
            tot_lines: 0,
            tot_comments: 0,
            tot_blanks: 0,
            tot_code: 0,
            tot_usafe: 0
        }
    }

    pub fn fill_from(&mut self) {
        debugln!("executing; fill_from; cfg={:?}", self.cfg);
        for path in &self.cfg.to_count {
            debugln!("iter; path={:?};", path);
            if let Some(f) = path.to_str() {
                let files = fsutil::get_all_files(f, &self.cfg.exclude);

                for file in files {
                    debugln!("iter; file={:?};", file);
                    let extension = match Path::new(&file).extension() {
                        Some(result) => {
                            if let Some(ref exts) = self.cfg.exts {
                                if !exts.contains(&result.to_str().unwrap_or("")) { continue }
                            }
                            result.to_str().unwrap()
                        },
                        None => continue,
                    };

                    debugln!("found extension: {:?}", extension);
                    if let Some(pos_lang) = Language::from_ext(extension) {
                        debugln!("Extension is valid");
                        let mut found = false;
                        debugln!("Searching for previous entries of that type");
                        for l in self.counts.iter_mut() {
                            if l.lang.extension() == extension {
                                debugln!("Found");
                                found = true;
                                l.add_file(PathBuf::from(&file));
                                break;
                            }
                        }
                        if !found {
                            debugln!("Not found, creating new");
                            let mut c = Count::new(pos_lang, self.cfg.thousands);
                            c.add_file(PathBuf::from(&file));
                            self.counts.push(c);
                        }
                    } else {
                        debugln!("extension wasn't valid");
                    }
                }
            } else {
                debugln!("path couldn't be converted to a str");
            }
        }
    }

    pub fn count(&mut self) -> CliResult<()> {
        for count in self.counts.iter_mut() {
            debugln!("iter; count={:?};", count);
            let re = if let Some(kw) = count.lang.unsafe_keyword() {
                regex!(&*format!("[\\[\\] \\{{\\}}]{}[\\[\\] \\{{\\}}\n]", kw))
            } else {
                regex!("")
            };
            for file in count.files.iter() {
                debugln!("iter; file={:?};", file);
                let mut buffer = String::new();

                let mut file_ref = cli_try!(File::open(&file));

                match self.cfg.utf8_rule {
                    Utf8Rule::Ignore => {
                        if let Err(..) = file_ref.read_to_string(&mut buffer) {
                            continue
                        }
                    },
                    Utf8Rule::Lossy => {
                        let mut vec_buf = vec![];
                        cli_try!(file_ref.read_to_end(&mut vec_buf));
                        buffer = String::from_utf8_lossy(&vec_buf).into_owned();
                    },
                    Utf8Rule::Strict => {
                        cli_try!(file_ref.read_to_string(&mut buffer));
                    }
                } 
                let mut is_in_comments = false;
                let mut is_in_unsafe = false;
                let mut bracket_count: i64 = 0;

                'new_line: for line in buffer.lines() {
                    let line = line.trim();
                    debugln!("iter; line={:?};", line);
                    count.lines += 1;

                    if is_in_comments {
                        debugln!("still in comments");
                        if line.contains(count.multi_end().unwrap()) {
                            debugln!("line contained ending comment, stopping comments");
                            is_in_comments = false;
                        }
                        count.comments += 1;
                        continue;
                    }
                    debugln!("not in comments");

                    if line.trim().is_empty() {
                        debugln!("line was empty");
                        count.blanks += 1;
                        continue;
                    }
                    debugln!("Line isn't empty");

                    if let Some(ms) = count.multi_start() {
                        debugln!("This file type has a multi start of: {:?}", ms);
                        if line.starts_with(ms) {
                            debugln!("line starts with multi comment");
                            count.comments += 1;
                            is_in_comments = !line.contains(count.multi_end().unwrap());
                            debugln!("line also contained a multi end: {:?}", is_in_comments);
                            continue;
                        } else if line.contains(ms) {
                            debugln!("line contains a multi start");
                            is_in_comments = !line.contains(count.multi_end().unwrap());
                            debugln!("line also contained a multi end: {:?}", is_in_comments);
                            if is_in_comments { continue; }
                        }
                    } else {
                        debugln!("No multi line comments for this type");
                    }
                    debugln!("No multi line comments for this line");

                    if let Some(single_comments) = count.single() {
                        debugln!("This type has single line comments: {:?}", single_comments);
                        for single in single_comments {
                            if line.starts_with(single) {
                                debugln!("Line started with a comment");
                                count.comments += 1;
                                continue 'new_line;
                            } else {
                                debugln!("Line dind't start with a comment");
                            }
                        }
                    } else {
                        debugln!("No single line comments for this type");
                    }

                    if self.cfg.usafe {
                        debugln!("Calculating --unsafe-statistics");
                        if count.lang.is_unsafe() {
                            debugln!("The language is not safe");
                            if let Some(kw) = count.lang.unsafe_keyword() {
                                debugln!("There is a keyword: {}", kw);
                                debugln!("line={:?}", line);
                                if re.is_match(line) {
                                    debugln!("It contained the keyword; usafe_line={:?}", line);
                                    count.usafe += 1;
                                    let after_usafe = line.split(kw).collect::<Vec<_>>()[1];
                                    debugln!("after_usafe={:?}", after_usafe);
                                    is_in_unsafe = Counts::in_unsafe(after_usafe, None);
                                    debugln!("after counting brackets; is_in_unsafe={:?}; bracket_count={:?}", is_in_unsafe, bracket_count);
                                } else if is_in_unsafe {
                                    debugln!("It didn't contain the keyword, but we are still in unsafe");
                                    count.usafe += 1;
                                    is_in_unsafe = Counts::in_unsafe(line, Some(bracket_count));
                                    debugln!("after counting brackets; is_in_unsafe={:?}; bracket_count={:?}", is_in_unsafe, bracket_count);
                                } else {
                                    debugln!("It didn't contain the keyword, and we are not in unsafe");
                                }

                                if bracket_count < 0 {
                                    debugln!("bracket_count < 0; resetting");
                                    bracket_count = 0
                                }
                            } else {
                                debugln!("Language is unsafe, incing the count");
                                count.usafe += 1;
                            }
                        }
                    }
                    count.code += 1;
                }
            }
            self.tot          += count.files.len();
            self.tot_lines    += count.lines;
            self.tot_comments += count.comments;
            self.tot_blanks   += count.blanks;
            self.tot_code     += count.code;
            self.tot_usafe    += count.usafe;
        }

        Ok(())
    }

    pub fn write_results(&mut self) -> CliResult<()> {
        let mut w = TabWriter::new(vec![]);
        cli_try!(write!(w, "\tLanguage\tFiles\tLines\tBlanks\tComments\tCode{}\n", if self.cfg.usafe {"\tUnsafe (%)"} else {""}));
        cli_try!(write!(w, "\t--------\t-----\t-----\t------\t--------\t----{}\n", if self.cfg.usafe {"\t----------"} else {""}));
        for count in &self.counts {
            if !self.cfg.usafe {
                cli_try!(write!(w, "\t{}\n", count));
            } else {
                let usafe_per = (count.usafe as f64 / count.code as f64) * 100.00f64;
                cli_try!(write!(w, "\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                    count.lang.name(),
                    count.total_files(),
                    count.lines(),
                    count.blanks(),
                    count.comments(),
                    count.code(),
                    if usafe_per == 00f64 { "".to_owned() } else { format!("{} ({:.2}%)", count.usafe(), usafe_per) }
                    ));
            }
        }
        cli_try!(write!(w, "\t--------\t-----\t-----\t------\t--------\t----{}\n", if self.cfg.usafe { "\t----------"}else{""}));
        cli_try!(write!(w, "{}\t\t{}\t{}\t{}\t{}\t{}{}\n",
            "Totals:",
            fmt::format_number(self.tot as u64, self.cfg.thousands),
            fmt::format_number(self.tot_lines, self.cfg.thousands),
            fmt::format_number(self.tot_blanks, self.cfg.thousands),
            fmt::format_number(self.tot_comments, self.cfg.thousands),
            fmt::format_number(self.tot_code, self.cfg.thousands),
            if self.cfg.usafe {
                format!("\t{} ({:.2}%)", fmt::format_number(self.tot_usafe, self.cfg.thousands), ((self.tot_usafe as f64 / self.tot_code as f64) * 100.00f64) as f64)
            } else {
                "".to_owned()
            }));

        cli_try!(w.flush());

        verboseln!(self.cfg, "{} {}", Format::Good("Displaying"), "the results:");
        if self.tot > 0 {
            write!(io::stdout(), "{}", String::from_utf8(w.unwrap()).ok().expect("failed to get valid UTF-8 String")).ok().expect("failed to write output");
        } else {
            println!("\n\tNo source files were found matching the specified criteria");
        }
        Ok(())
    }

    fn in_unsafe(line: &str, count: Option<i64>) -> bool {
        let mut b: i64 = count.unwrap_or(0);
        for c in line.chars() {
            match c {
                '{' => b += 1,
                '}' => b -= 1,
                _   => (),
            }
        }
        b > 0
    }
}