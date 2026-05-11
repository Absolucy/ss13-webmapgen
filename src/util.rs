// SPDX-License-Identifier: MPL-2.0

pub fn thread_safe_print(meow: impl AsRef<str>) {
	use std::io::Write;

	let mut stdout = std::io::stdout().lock();
	let _ = writeln!(stdout, "{}", meow.as_ref());
}

pub fn thread_safe_print_err(meow: impl AsRef<str>) {
	use std::io::Write;

	let mut stderr = std::io::stderr().lock();
	let _ = writeln!(stderr, "{}", meow.as_ref());
}
