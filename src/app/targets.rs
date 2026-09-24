use cargo_metadata::{MetadataCommand, Target as CargoTarget};
use std::{collections::HashMap, fs, path::Path};

#[derive(Debug, Clone)]
pub(crate) struct Target {
    pub(crate) package_name: String,
    pub(crate) kind: String,
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) description: Option<String>,
    pub(crate) required_features: Vec<String>,
}
pub(super) fn discover_targets(project_path: &Path) -> Vec<Target> {
    let Ok(metadata) = MetadataCommand::new()
        .current_dir(project_path)
        .no_deps()
        .exec()
    else {
        return Vec::new();
    };

    let mut targets = metadata
        .packages
        .iter()
        .flat_map(|package| {
            let manifest_descriptions =
                load_target_descriptions(package.manifest_path.as_std_path());
            package.targets.iter().filter_map(move |target| {
                select_target_kind(target).map(|kind| Target {
                    package_name: package.name.to_string(),
                    kind: kind.to_string(),
                    name: target.name.clone(),
                    path: target
                        .src_path
                        .strip_prefix(project_path)
                        .map(|path| path.to_string())
                        .unwrap_or_else(|_| target.src_path.to_string()),
                    description: manifest_descriptions
                        .get(&(kind.to_string(), target.name.clone()))
                        .cloned()
                        .flatten(),
                    required_features: target.required_features.clone(),
                })
            })
        })
        .collect::<Vec<_>>();

    targets.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.kind.cmp(&b.kind))
    });
    targets
}

pub(super) fn load_target_descriptions(
    manifest_path: &Path,
) -> HashMap<(String, String), Option<String>> {
    let Ok(cargo_toml) = fs::read_to_string(manifest_path) else {
        return HashMap::new();
    };
    let Ok(value) = toml::from_str::<toml::Value>(&cargo_toml) else {
        return HashMap::new();
    };

    let mut descriptions = HashMap::new();
    for (manifest_key, target_kind) in [
        ("bin", "bin"),
        ("example", "example"),
        ("test", "test"),
        ("bench", "bench"),
    ] {
        if let Some(items) = value.get(manifest_key).and_then(|items| items.as_array()) {
            for item in items {
                let Some(table) = item.as_table() else {
                    continue;
                };
                let Some(name) = table.get("name").and_then(|name| name.as_str()) else {
                    continue;
                };
                let description = table
                    .get("description")
                    .and_then(|description| description.as_str())
                    .map(str::trim)
                    .filter(|description| !description.is_empty())
                    .map(ToOwned::to_owned);
                descriptions.insert((target_kind.to_string(), name.to_string()), description);
            }
        }

        let metadata_examples = value
            .get("package")
            .and_then(|package| package.get("metadata"))
            .and_then(|metadata| metadata.get(manifest_key))
            .and_then(|targets| targets.as_table());

        if let Some(targets) = metadata_examples {
            for (name, target_metadata) in targets {
                let description = target_metadata
                    .get("description")
                    .and_then(|description| description.as_str())
                    .map(str::trim)
                    .filter(|description| !description.is_empty())
                    .map(ToOwned::to_owned);
                if description.is_some() {
                    descriptions.insert((target_kind.to_string(), name.to_string()), description);
                }
            }
        }
    }

    descriptions
}

pub(super) fn select_target_kind(target: &CargoTarget) -> Option<&'static str> {
    if target.is_bin() {
        Some("bin")
    } else if target.is_example() {
        Some("example")
    } else if target.is_test() {
        Some("test")
    } else if target.is_bench() {
        Some("bench")
    } else {
        None
    }
}
