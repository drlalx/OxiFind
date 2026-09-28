mod app;
mod elevation;
mod ntfs;
mod search;

use std::{env, error::Error, ffi::OsString, io, path::PathBuf};

use app::IndexScope;

fn main() {
    let result = index_scope_from_args(env::args_os().skip(1))
        .map_err(Box::<dyn Error + Send + Sync>::from)
        .and_then(app::run);
    if let Err(error) = result {
        eprintln!("Application failed: {error}");
        std::process::exit(1);
    }
}

/// Without an argument every local drive is indexed; otherwise the given directory or drive.
fn index_scope_from_args(arguments: impl IntoIterator<Item = OsString>) -> io::Result<IndexScope> {
    let mut arguments = arguments.into_iter();
    let scope = arguments.next().map_or(IndexScope::LocalDrives, |root| {
        IndexScope::Root(PathBuf::from(root))
    });

    if arguments.next().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Usage: oxidex [INDEX_ROOT]",
        ));
    }

    Ok(scope)
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, path::PathBuf};

    use super::{IndexScope, index_scope_from_args};

    #[test]
    fn indexes_all_local_drives_without_an_argument() {
        let scope = index_scope_from_args([]).unwrap();

        assert_eq!(scope, IndexScope::LocalDrives);
    }

    #[test]
    fn uses_the_single_argument_as_the_index_root() {
        let scope = index_scope_from_args([OsString::from(r"C:\search-root")]).unwrap();

        assert_eq!(scope, IndexScope::Root(PathBuf::from(r"C:\search-root")));
    }

    #[test]
    fn rejects_more_than_one_index_root() {
        let error = index_scope_from_args([
            OsString::from(r"C:\first-root"),
            OsString::from(r"D:\second-root"),
        ])
        .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }
}
