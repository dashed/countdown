#[macro_use]
extern crate chomp;
extern crate clap;
extern crate timer;
extern crate chrono;
#[macro_use]
extern crate chan;
extern crate chan_signal;


use std::thread;
use std::sync::{Arc, Mutex};
use std::io::{self, Write};
use std::process::Command;
use std::ascii::{AsciiExt};
use std::process;

use chan_signal::Signal;

use chrono::offset::local::Local;

use clap::{Arg, App};

// TODO: reorg this
use chomp::{SimpleResult, Error};
use chomp::primitives::{InputBuffer};
use chomp::{Input, U8Result, parse_only};

use chomp::{token};
use chomp::parsers::{string, eof};
use chomp::combinators::{or, many_till, many1, skip_many};
use chomp::ascii::{decimal};

// clear line and turn off cursor
const CLEAR_LINE: &'static str = "\r\x1b[?25l";

fn main() {

    let cmd_matches = App::new("gtdtxt")
        .version("v0.2.0 (semver.org)") // semver semantics
        .about("Countdown or countup program")
        .author("Alberto Leal <mailforalberto@gmail.com> (github.com/dashed)")
        .arg(
            Arg::with_name("note")
            .help("Attached note.")
            .short("note")
            .long("note")
            .required(false)
            .takes_value(true)
            .validator(|note| {
                let note = note.trim();
                if note.len() <= 0 {
                    return Err(String::from("invalid note"));
                }
                return Ok(());
            })
        )
        .arg(
            Arg::with_name("count-up")
            .help("Count up.")
            .short("u")
            .long("count-up")
            .required(false)
        )
        .arg(
            Arg::with_name("time-length")
            .help("Time length.")
            .required(false)
            .index(1)
            .validator(|time_len| {
                let time_len = time_len.trim();
                if time_len.len() <= 0 {
                    return Err(String::from("invalid time length"));
                } else {
                    return Ok(());
                }
            })
        ).get_matches();


    let wait_for = {

        if !cmd_matches.is_present("time-length") {
            0
        } else {
            let time_length: String = cmd_matches.value_of("time-length")
                                                        .unwrap()
                                                        .trim()
                                                        .to_string();

            let wait_for = match parse_only(|i| time_length_parser(i), time_length.as_bytes()) {
                Ok(result) => {
                    result
                },
                Err(e) => {
                    println!("Unable to parse: {}", time_length);
                    process::exit(1);
                    // TODO: refactor
                    // panic!("{:?}", e);
                }
            };

            wait_for
        }

    };



    let count_request = if cmd_matches.is_present("count-up") {
        println!("Counting up...");
        Counter::CountUp
    } else {
        println!("Counting down {}", Timerange::new(wait_for).print());
        Counter::CountDown(wait_for)
    };

    println!("Began counting at {}", Local::now().naive_local().format("%B %e, %Y %-l:%M:%S %p"));

    if cmd_matches.is_present("note") {
        println!("Note: {}", cmd_matches.value_of("note").unwrap());
    }

    let count_up_seconds = Arc::new(Mutex::new(0));
    let count_up_seconds2 = count_up_seconds.clone();

    // Signal gets a value when the OS sent a INT or TERM signal.
    let signal = chan_signal::notify(&[Signal::INT, Signal::TERM]);
    // When our work is complete, send a sentinel value on `sdone`.
    let (sdone, rdone) = chan::sync(0);
    // Run work.
    ::std::thread::spawn(move || run(sdone, count_request, count_up_seconds));

    // Wait for a signal or for work to be done.
    chan_select! {
        signal.recv() -> signal => {
            // println!("received signal: {:?}", signal);

            // we capture SIGINT or SIGTERM so we can turn cursor back on
            println!("\x1b[?25h");
            io::stdout().flush().unwrap();

            let count_up = *count_up_seconds2.lock().unwrap();

            // if count_up < wait_for {
            //     // display time progressed
            //     println!("Counted up {}", Timerange::new(count_up).print());
            // }
        },
        rdone.recv() => {
            println!("Program completed normally.");
        }
    }

}

enum Counter {
    CountUp,
    CountDown(u64)
}

fn run(_sdone: chan::Sender<()>, count_request: Counter, count_up_seconds: Arc<Mutex<u64>>) {

    let timer = timer::Timer::new();

    // Start counting up.
    let guard = {
      let count_up_seconds = count_up_seconds.clone();
      timer.schedule_repeating(chrono::Duration::seconds(1), move || {
        *count_up_seconds.lock().unwrap() += 1;
      })
    };

    let mut seconds_passed = *count_up_seconds.lock().unwrap();

    print!("\x1b[?25l");
    io::stdout().flush().unwrap();

    let start = match count_request {
        Counter::CountUp => 0,
        Counter::CountDown(wait_for) => wait_for - seconds_passed
    };

    let mut out = prep_pretty(Timerange::new(start).print(), &count_request);
    print_line(out.clone());


    loop {

        match count_request {
            Counter::CountUp => {},
            Counter::CountDown(wait_for) => {
                if wait_for <= 0 {
                    break;
                }
            }
        }

        // Sleep for 250 ms (250000000 nanoseconds)
        thread::sleep(std::time::Duration::new(0, 250000000));

        // Fetch counts
        let count_result = *count_up_seconds.lock().unwrap();

        if count_result <= seconds_passed {
            continue;
        }

        seconds_passed = count_result;

        // amount of time remaining or passing
        let diff: u64 = match count_request {
            Counter::CountUp => seconds_passed,
            Counter::CountDown(wait_for) => {
                if seconds_passed > wait_for {
                    0
                } else {
                    wait_for - seconds_passed
                }
            }
        };


        let new_out = prep_pretty(Timerange::new(diff).print(), &count_request);

        let filler: String = if out.len() >= new_out.len() {
            String::from_utf8(vec![b' '; out.len() - new_out.len()]).ok().unwrap()
        } else{
            "".to_string()
        };

        print_line(format!("{}{}", new_out, filler));

        out = new_out;

        // guard test
        match count_request {
            Counter::CountUp => {},
            Counter::CountDown(wait_for) => {
                if seconds_passed >= wait_for {
                    break;
                }
            }
        };


    }


    // turn cursor back on
    println!("\x1b[?25h");

    // Now drop the guard. This should stop the timer.
    drop(guard);

    println!("Finished counting at {}", Local::now().naive_local().format("%B %e, %Y %-l:%M:%S %p"));


    // Alarm clock
    loop {
        let output = Command::new("sh")
                             .arg("-c")
                             // .arg("say 'beep'")
                             .arg("afplay /System/Library/Sounds/Glass.aiff")
                             .output()
                             .unwrap_or_else(|e| { panic!("failed to execute process: {}", e) });
    }

}

struct Timerange {
    range: u64
}

impl Timerange {

    fn new(range: u64) -> Timerange {
        Timerange {
            range: range
        }
    }

    fn floor_time_unit(&self) -> (u64, u64, String) {

        let sec_per_minute: f64 = 60f64;
        let sec_per_hour: f64 = sec_per_minute * 60f64;
        let sec_per_day: f64 = sec_per_hour * 24f64;
        let sec_per_month: f64 = sec_per_day * 30f64;
        let sec_per_year: f64 = sec_per_day * 365f64;

        let mut elapsed = self.range as f64;
        let mut remainder: f64 = 0f64;
        let unit;

        if elapsed < sec_per_minute {
            unit = "second";
        } else if elapsed < sec_per_hour {
            remainder = elapsed % sec_per_minute;
            elapsed = (elapsed / sec_per_minute).floor();
            unit = "minute"
        } else if elapsed < sec_per_day {
            remainder = elapsed % sec_per_hour;
            elapsed = (elapsed / sec_per_hour).floor();
            unit = "hour"
        } else if elapsed < sec_per_month {
            remainder = elapsed % sec_per_day;
            elapsed = (elapsed / sec_per_day).floor();
            unit = "day"
        } else if elapsed < sec_per_year {
            remainder = elapsed % sec_per_month;
            elapsed = (elapsed / sec_per_month).floor();
            unit = "month"
        } else {
            remainder = elapsed % sec_per_year;
            elapsed = (elapsed / sec_per_year).floor();
            unit = "year"
        }

        // pluralize
        let unit = if elapsed <= 1f64 {
            format!("{}", unit)
        } else {
            format!("{}s", unit)
        };

        let elapsed = elapsed as u64;
        let remainder = remainder as u64;

        return (elapsed, remainder, unit);
    }

    fn print(&self) -> String {

        let (elapsed, remainder, unit) = self.floor_time_unit();

        if remainder <= 0 {
            return format!("{} {}", elapsed, unit);
        }

        let pretty_remainder = Timerange::new(remainder).print();

        if remainder < 60 {
            return format!("{} {} and {}", elapsed, unit, pretty_remainder);
        }


        return format!("{} {} {}", elapsed, unit, pretty_remainder);

    }
}

/* time range parsers */

fn time_length_parser(i: Input<u8>) -> U8Result<u64> {
    parse!{i;

        skip_many(|i| space_or_tab(i));

        let range = time_range_list() <|> decimal();

        let nothing: Vec<()> = many_till(|i| space_or_tab(i), |i| eof(i));

        ret range
    }
}

fn time_range_list(i: Input<u8>) -> U8Result<u64> {
    parse!{i;

        let time: Vec<u64> = many1(|i| parse!{
            i;
            skip_many(|i| space_or_tab(i));
            let range = time_range();
            ret range
        });

        ret {
            let time = time.iter().fold(0, |mut sum, &val| {sum += val; sum});
            time
        }
    }
}

fn time_range(i: Input<u8>) -> U8Result<u64> {
    parse!{i;

        let range: u64 = decimal();

        skip_many(|i| space_or_tab(i));

        let multiplier = time_range_unit_minutes() <|>
            time_range_unit_hours() <|>
            time_range_unit_seconds();

        ret {
            range * multiplier
        }
    }
}

fn time_range_unit_seconds(i: Input<u8>) -> U8Result<u64> {
    parse!{i;

        string_ignore_case("seconds".as_bytes()) <|>
        string_ignore_case("second".as_bytes()) <|>
        string_ignore_case("secs".as_bytes()) <|>
        string_ignore_case("sec".as_bytes()) <|>
        string_ignore_case("s".as_bytes());

        ret 1
    }
}

fn time_range_unit_minutes(i: Input<u8>) -> U8Result<u64> {
    parse!{i;

        string_ignore_case("minutes".as_bytes()) <|>
        string_ignore_case("minute".as_bytes()) <|>
        string_ignore_case("mins".as_bytes()) <|>
        string_ignore_case("min".as_bytes()) <|>
        string_ignore_case("m".as_bytes());

        // 60 seconds in a minute
        ret 60
    }
}

fn time_range_unit_hours(i: Input<u8>) -> U8Result<u64> {
    parse!{i;

        string_ignore_case("hours".as_bytes()) <|>
        string_ignore_case("hour".as_bytes()) <|>
        string_ignore_case("hrs".as_bytes()) <|>
        string_ignore_case("hr".as_bytes()) <|>
        string_ignore_case("h".as_bytes());

        // 3600 seconds in an hour
        ret 3600
    }
}

/* helper */

fn space_or_tab(input: Input<u8>) -> U8Result<()> {
    parse!{input;
        or(
            |i| token(i, b' '),
            |i| token(i, b'\t')
        );
        ret ()
    }
}

fn string_ignore_case<'a>(i: Input<'a, u8>, s: &[u8])
    -> SimpleResult<'a, u8, &'a [u8]> {
    let b = i.buffer();

    if s.len() > b.len() {
        return i.incomplete(s.len() - b.len());
    }

    let d = &b[..s.len()];

    for j in 0..s.len() {

        if !(s[j]).eq_ignore_ascii_case(&(d[j])) {
            return i.replace(&b[j..]).err(Error::expected(d[j]))
        }
    }

    i.replace(&b[s.len()..]).ret(d)
}

fn prep_pretty(left: String, count_request: &Counter) -> String {
    match count_request {
        &Counter::CountUp => {
            format!("{} passed", left)
        },
        &Counter::CountDown(wait_for) => {
            format!("{} left", left)
        }
    }
}

fn print_line(string: String) {
    print!("{}{}", CLEAR_LINE, string);
    io::stdout().flush().unwrap();
}
