use gpui_kit::{AssetSource, Result, SharedString, assets};
use std::borrow::Cow;

assets::icon_assets!(ExtraIcons, [Spool, Boxes, Activity, CircleDashed]);

// Embed web controls too: no CDN requests or asset base-path assumptions.
#[cfg(any(test, target_family = "wasm"))]
assets::icon_assets!(
    WebIcons,
    [
        ArrowDown,
        ArrowLeft,
        ArrowRight,
        ArrowUp,
        Asterisk,
        Calendar,
        Check,
        ChevronDown,
        ChevronLeft,
        ChevronRight,
        ChevronUp,
        ChevronsUpDown,
        CircleAlert,
        CircleCheck,
        CircleX,
        Close,
        Copy,
        Dash,
        Delete,
        Ellipsis,
        EllipsisVertical,
        ExternalLink,
        Eye,
        EyeOff,
        File,
        Folder,
        Inbox,
        Info,
        Loader,
        LoaderCircle,
        Maximize,
        Minimize,
        Minus,
        PanelLeftClose,
        PanelLeftOpen,
        Pause,
        Play,
        Plus,
        RefreshCw,
        RotateCw,
        Search,
        Settings,
        TriangleAlert,
    ]
);

pub(super) struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        #[cfg(not(target_family = "wasm"))]
        {
            assets::Assets.load(path)
        }
        #[cfg(target_family = "wasm")]
        {
            WebIcons.load(path)
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        #[cfg(not(target_family = "wasm"))]
        let mut paths = assets::Assets.list(path)?;
        #[cfg(target_family = "wasm")]
        let mut paths = WebIcons.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::{AppAssets, AssetSource, WebIcons};

    #[test]
    fn web_search_controls_and_empty_state_icons_are_bundled() {
        for path in [
            "icons/search.svg",
            "icons/chevron-down.svg",
            "icons/check.svg",
            "icons/close.svg",
            "icons/inbox.svg",
        ] {
            assert!(WebIcons.load(path).unwrap().is_some(), "{path}");
        }
    }

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
