use gpui_kit::{AssetSource, Result, SharedString, assets};
use std::borrow::Cow;

assets::icon_assets!(ExtraIcons, [Spool, Boxes, Activity, CircleDashed]);

pub(super) struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::{AppAssets, AssetSource};

    #[test]
    fn navigation_thread_and_default_control_icons_are_bundled() {
        for path in [
            "icons/spool.svg",
            "icons/boxes.svg",
            "icons/activity.svg",
            "icons/circle-dashed.svg",
            "icons/play.svg",
            "icons/pause.svg",
            "icons/plus.svg",
            "icons/minus.svg",
            "icons/maximize.svg",
        ] {
            assert!(AppAssets.load(path).unwrap().is_some(), "{path}");
        }
        assert!(
            AppAssets
                .list("icons/")
                .unwrap()
                .iter()
                .any(|p| p == "icons/spool.svg")
        );
    }
}
