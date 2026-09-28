mod app;
mod ntfs;
mod search;

use std::{env, error::Error, ffi::OsString, io, path::PathBuf};

fn main() {
    let result = index_root_from_args(env::args_os().skip(1))
        .map_err(Box::<dyn Error + Send + Sync>::from)
        .and_then(app::run);
    if let Err(error) = result {
        eprintln!("Application failed: {error}");
        std::process::exit(1);
    }
}

fn index_root_from_args(arguments: impl IntoIterator<Item = OsString>) -> io::Result<PathBuf> {
    let mut arguments = arguments.into_iter();
    let root = arguments
        .next()
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(env::current_dir)?;

    if arguments.next().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Usage: everything_lite [INDEX_ROOT]",
        ));
    }

    Ok(root)
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, path::PathBuf};

    use super::index_root_from_args;

    #[test]
    fn uses_the_single_argument_as_the_index_root() {
        let root = index_root_from_args([OsString::from(r"C:\search-root")]).unwrap();

        assert_eq!(root, PathBuf::from(r"C:\search-root"));
    }

    #[test]
    fn rejects_more_than_one_index_root() {
        let error = index_root_from_args([
            OsString::from(r"C:\first-root"),
            OsString::from(r"D:\second-root"),
        ])
        .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }
}
