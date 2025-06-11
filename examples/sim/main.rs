mod dummy_terminal;
mod nonblocknoecho;
mod popup_terminal;

use crate::dummy_terminal::DummyTerminal;
use crate::popup_terminal::PopupTerminal;
use getopts::Options;
use simmerv::Emulator;
use simmerv::terminal::Terminal;
use std::env;
use std::fs::File;
use std::io::Read;

enum TerminalType {
    PopupTerminal,
    DummyTerminal,
}

fn print_usage(program: &str, opts: &Options) {
    let usage = format!("Usage: {program} program_file [options]");
    print!("{}", opts.usage(&usage));
}

fn get_terminal(terminal_type: &TerminalType) -> Box<dyn Terminal> {
    match terminal_type {
        TerminalType::PopupTerminal => Box::new(PopupTerminal::new()),
        TerminalType::DummyTerminal => Box::new(DummyTerminal::new()),
    }
}

fn main() -> std::io::Result<()> {
    env_logger::init();

    let args: Vec<String> = env::args().collect();
    let program = args[0].clone();

    let mut opts = Options::new();
    opts.optopt("f", "fs", "File system image file", "xv6/fs.img");
    opts.optopt("d", "dtb", "Device tree file", "linux/dtb");
    opts.optflag("n", "no_terminal", "No popup terminal");
    opts.optflag("h", "help", "Show this help menu");
    opts.optflag("t", "trace", "Run with trace");
    opts.optflag(
        "p",
        "page_cache",
        "Enable experimental page cache optimization",
    );

    let matches = match opts.parse(&args[2..]) {
        Ok(m) => m,
        Err(f) => {
            println!("{f}");
            print_usage(&program, &opts);
            // @TODO: throw error?
            return Ok(());
        }
    };

    if matches.opt_present("h") {
        print_usage(&program, &opts);
        return Ok(());
    }

    if args.len() < 2 {
        print_usage(&program, &opts);
        // @TODO: throw error?
        return Ok(());
    }

    let fs_contents = match matches.opt_str("f") {
        Some(path) => {
            let mut file = File::open(path)?;
            let mut contents = vec![];
            file.read_to_end(&mut contents)?;
            contents
        }
        None => vec![],
    };

    let mut has_dtb = false;
    let dtb_contents = match matches.opt_str("d") {
        Some(path) => {
            has_dtb = true;
            let mut file = File::open(path)?;
            let mut contents = vec![];
            file.read_to_end(&mut contents)?;
            contents
        }
        None => vec![],
    };

    let elf_filename = args[1].clone();
    let mut elf_file = File::open(elf_filename)?;
    let mut elf_contents = vec![];
    elf_file.read_to_end(&mut elf_contents)?;

    let initrd_filename = args[2].clone();
    let mut initrd_file = File::open(initrd_filename)?;
    let mut initrd_contents = vec![];
    initrd_file.read_to_end(&mut initrd_contents)?;

    let terminal_type = if matches.opt_present("n") {
        TerminalType::DummyTerminal
    } else {
        TerminalType::PopupTerminal
    };

    let mut emulator = Emulator::new(get_terminal(&terminal_type));
    emulator.setup_program(elf_contents);
    emulator.setup_filesystem(fs_contents);
    if has_dtb {
        emulator.setup_dtb(&dtb_contents);
    }
    if matches.opt_present("p") {
        emulator.enable_page_cache(true);
    }

    for (j, b) in initrd_contents.iter().enumerate() {
        if emulator
            .cpu
            .get_mut_mmu()
            .store_phys_u8(0x86000000 + j as u64, *b)
            .is_err()
        {
            panic!(
                "Program doesn't fit in memory: 0x{:016x}",
                0x86000000 + j as u64
            );
        }
    }

    emulator.run(matches.opt_present("t"));
    Ok(())
}
