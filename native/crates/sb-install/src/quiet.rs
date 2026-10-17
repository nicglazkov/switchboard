//! --quiet: the same flows with no window. One line per step on stdout, problems on stderr, and the exit code says
//! how it went: 0 done, 1 failed (nothing changed unless the line says otherwise), 2 bad arguments, 3 refused
//! (a newer version is installed).

use crate::Args;
use crate::brand::NAME;
use crate::flow::{self, Event};
use crate::platform;

pub fn run(args: &Args) -> i32 {
    let p = platform::native();
    let opts = &args.opts;
    let mut print = |e: Event| match e {
        Event::Start(t) => println!("{}: {}", t.tag(), t.label()),
        Event::Done(t, d) if !d.is_empty() => println!("{}: done, {d}", t.tag()),
        Event::Done(..) => {}
        Event::Stuck => {
            if opts.force_quit {
                println!("quit: {NAME} did not quit when asked; stopping it (--force-quit)");
            } else {
                eprintln!("quit: {NAME} did not quit when asked; pass --force-quit to stop it");
            }
        }
    };
    let force = opts.force_quit;
    let mut ask_force = move || force;

    if args.uninstall {
        let Some((_, app)) = flow::find_installed(p.as_ref(), opts.dest.as_deref()) else {
            eprintln!("uninstall: {NAME} is not installed here");
            return 1;
        };
        let plan = flow::uninstall_plan(p.as_ref(), &app, opts.remove_data);
        println!("uninstall: {NAME} at {}", app.display());
        for i in &plan.keep {
            println!("keep: {}", i.path.display());
        }
        return match flow::uninstall(p.as_ref(), &plan, &mut ask_force, &mut print) {
            Ok(out) => {
                out.notes.iter().for_each(|n| println!("note: {n}"));
                println!("done: {NAME} is uninstalled");
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        };
    }

    let setup = match flow::prepare(p.as_ref(), opts) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    println!("{}", setup.title());
    if let Some(no) = setup.refusal() {
        eprintln!("refused: {no}");
        return 3;
    }
    let where_ = match &setup.why_dir {
        Some(why) => format!("{} ({why})", setup.dir.path.display()),
        None => setup.dir.path.display().to_string(),
    };
    println!("into: {where_}");
    if setup.chosen_before() {
        println!("first-run: chosen at an earlier install, left as they are");
    }
    match flow::install(p.as_ref(), &setup, opts.choices, &mut ask_force, &mut print) {
        Ok(out) => {
            out.notes.iter().for_each(|n| println!("note: {n}"));
            if opts.open {
                match p.open_app(&setup.dest) {
                    Ok(()) => println!("open: {NAME} is starting"),
                    Err(e) => println!("note: could not open {NAME}: {e}"),
                }
            }
            if opts.eject {
                match setup.disk_image.as_ref().map(|i| p.eject_after_exit(i)) {
                    Some(Ok(())) => println!("eject: the disk image goes to the Trash once this exits"),
                    Some(Err(e)) => println!("note: could not eject the disk image: {e}"),
                    None => println!("note: not run from a disk image, nothing to eject"),
                }
            }
            println!("done: {NAME} {} is in {}", setup.version, setup.dir.path.display());
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}
