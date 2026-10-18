//! Full-bleed adaptive launcher resources (all supported devices are API 29+).
use pack_asset_compiler::resource_internal_types::FileResource;

pub fn resources(png: Vec<u8>) -> Vec<FileResource> {
    let file = |name: &str, contents: Vec<u8>| FileResource {
        subdirectory: "drawable".into(), name: name.into(), resource_id: 0, contents,
    };
    vec![
        file("icon_art.png", png),
        file("icon_clear.png", include_bytes!("../assets/transparent.png").to_vec()),
        file("icon.xml", br##"<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">
<background android:drawable="@drawable/icon_art" />
<foreground android:drawable="@drawable/icon_clear" />
</adaptive-icon>"##.to_vec()),
    ]
}
