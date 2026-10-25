//! `zajil category`: the categories that choose a finished download's
//! folder (M4). Every change goes through [`CategoryStore::update`].

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Subcommand;
use zajil_core::category::{
    CategoriesFile, Category, CategoryError, CategoryStore, check_folder, normalize_extension,
    normalize_site,
};

use crate::fail;

#[derive(Debug, Subcommand)]
pub enum CategoryCommand {
    /// Show the categories, in the order they are tried.
    List,
    /// Add a category at the end of the list.
    Add {
        /// Its id: lowercase letters, digits, `-` or `_`.
        id: String,
        /// The name shown for it.
        #[arg(long, value_name = "NAME")]
        name: String,
    },
    /// Change a category.
    Set {
        id: String,
        /// A new name. For a built-in category, the translated name is
        /// shown until it has one of its own.
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
        /// The folder its downloads go to (an absolute path).
        #[arg(long, value_name = "FOLDER", conflicts_with = "no_folder")]
        folder: Option<PathBuf>,
        /// Its downloads stay in the folder they were given.
        #[arg(long)]
        no_folder: bool,
        /// File extensions to add, such as zip,7z.
        #[arg(long, value_name = "EXT", value_delimiter = ',')]
        add_ext: Vec<String>,
        /// File extensions to remove.
        #[arg(long, value_name = "EXT", value_delimiter = ',')]
        remove_ext: Vec<String>,
        /// Sites to add, such as example.com (its subdomains match too).
        #[arg(long, value_name = "SITE", value_delimiter = ',')]
        add_site: Vec<String>,
        /// Sites to remove.
        #[arg(long, value_name = "SITE", value_delimiter = ',')]
        remove_site: Vec<String>,
    },
    /// Remove a category. Files already downloaded stay where they are.
    Remove { id: String },
    /// Show which category a file name (and site) would go to.
    Which {
        /// A file name, such as photos.zip.
        name: String,
        /// The site it comes from, such as files.example.com.
        #[arg(long, value_name = "SITE")]
        site: Option<String>,
    },
}

/// The English name of a built-in category; the interface translates it
/// (key `category-<id>`). The command line stays in English in M4 (owner's
/// decision on the M4 plan).
pub fn display_name(c: &Category) -> String {
    if let Some(name) = &c.name {
        return name.clone();
    }
    match c.id.as_str() {
        "programs" => "Programs",
        "video" => "Videos",
        // The app's names since #118 (#121): Videos and Audio.
        "music" => "Audio",
        "documents" => "Documents",
        "archives" => "Archives",
        other => other,
    }
    .to_owned()
}

pub fn manage(store: &CategoryStore, command: CategoryCommand) -> ExitCode {
    match run(store, command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => fail(&err.to_string()),
    }
}

fn run(store: &CategoryStore, command: CategoryCommand) -> Result<(), CategoryError> {
    match command {
        CategoryCommand::List => {
            print_list(&store.load()?);
            Ok(())
        }
        CategoryCommand::Add { id, name } => {
            store.update(|file| file.add(&id, &name))?;
            println!("Added category {id}");
            Ok(())
        }
        CategoryCommand::Set {
            id,
            name,
            folder,
            no_folder,
            add_ext,
            remove_ext,
            add_site,
            remove_site,
        } => {
            // Check and normalize everything before the file is touched.
            let add_ext = add_ext
                .iter()
                .map(|e| normalize_extension(e))
                .collect::<Result<Vec<_>, _>>()?;
            let remove_ext = remove_ext
                .iter()
                .map(|e| normalize_extension(e))
                .collect::<Result<Vec<_>, _>>()?;
            let add_site = add_site
                .iter()
                .map(|s| normalize_site(s))
                .collect::<Result<Vec<_>, _>>()?;
            let remove_site = remove_site
                .iter()
                .map(|s| normalize_site(s))
                .collect::<Result<Vec<_>, _>>()?;
            let folder = folder.map(std::path::absolute).transpose()?;
            if let Some(folder) = &folder {
                check_folder(folder)?;
            }
            if let Some(name) = &name {
                zajil_core::category::check_name(name)?;
            }
            store.update(|file| {
                let c = file.get_mut(&id)?;
                if let Some(name) = name {
                    c.name = Some(name);
                }
                if no_folder {
                    c.folder = None;
                } else if let Some(folder) = folder {
                    c.folder = Some(folder);
                }
                c.extensions.retain(|e| !remove_ext.contains(e));
                for e in add_ext {
                    if !c.extensions.contains(&e) {
                        c.extensions.push(e);
                    }
                }
                c.sites.retain(|s| !remove_site.contains(s));
                for s in add_site {
                    if !c.sites.contains(&s) {
                        c.sites.push(s);
                    }
                }
                Ok(())
            })?;
            println!("Changed category {id}");
            Ok(())
        }
        CategoryCommand::Remove { id } => {
            store.update(|file| file.remove(&id))?;
            println!("Removed category {id}");
            Ok(())
        }
        CategoryCommand::Which { name, site } => {
            let site = site.map(|s| normalize_site(&s)).transpose()?;
            let file = store.load()?;
            match file.classify(&name, site.as_deref()) {
                Some(c) => println!("{} ({})", display_name(c), c.id),
                None => println!("No category"),
            }
            Ok(())
        }
    }
}

fn print_list(file: &CategoriesFile) {
    for c in &file.categories {
        println!("{} ({})", display_name(c), c.id);
        let folder = c
            .folder
            .as_ref()
            .map(|f| f.display().to_string())
            .unwrap_or_else(|| "(the folder each download is given)".to_owned());
        println!("  folder:     {folder}");
        if !c.sites.is_empty() {
            println!("  sites:      {}", c.sites.join(", "));
        }
        if !c.extensions.is_empty() {
            println!("  extensions: {}", c.extensions.join(", "));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zajil_core::category::CategoriesFile;

    /// #121: the command line names each built-in category as the app does
    /// in English (`category-<id>` in `locales/en/zajil.ftl`), so the two
    /// never drift apart again.
    #[test]
    fn built_in_names_are_the_app_s_english_ones() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../locales/en/zajil.ftl");
        let texts = std::fs::read_to_string(path).unwrap();
        let app_name = |id: &str| {
            let key = format!("category-{id} = ");
            texts
                .lines()
                .find_map(|l| l.strip_prefix(&key))
                .map(str::to_owned)
        };
        let built_in = CategoriesFile::built_in().categories;
        assert!(!built_in.is_empty());
        for c in &built_in {
            assert_eq!(Some(display_name(c)), app_name(&c.id), "{}", c.id);
        }
    }
}
