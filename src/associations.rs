//! Supported desktop associations; decoding remains in `formats`.
pub struct SupportedType {
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub mime: &'static str,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub uti: &'static str,
    // Used by the cross-platform packaging drift test, not the runtime setter.
    #[allow(dead_code)]
    pub extensions: &'static [&'static str],
}

pub fn supported_types() -> &'static [SupportedType] {
    TYPES
}

const TYPES: &[SupportedType] = &[
    SupportedType {
        mime: "image/jpeg",
        uti: "public.jpeg",
        extensions: &["jpg", "jpeg"],
    },
    SupportedType {
        mime: "image/png",
        uti: "public.png",
        extensions: &["png"],
    },
    SupportedType {
        mime: "image/tiff",
        uti: "public.tiff",
        extensions: &["tif", "tiff"],
    },
    SupportedType {
        mime: "image/bmp",
        uti: "com.microsoft.bmp",
        extensions: &["bmp"],
    },
    SupportedType {
        mime: "image/vnd.microsoft.icon",
        uti: "com.microsoft.ico",
        extensions: &["ico"],
    },
    SupportedType {
        mime: "image/vnd.microsoft.icon",
        uti: "com.microsoft.cur",
        extensions: &["cur"],
    },
    SupportedType {
        mime: "image/x-icns",
        uti: "com.apple.icns",
        extensions: &["icns"],
    },
    SupportedType {
        mime: "image/x-tga",
        uti: "com.truevision.tga-image",
        extensions: &["tga"],
    },
    SupportedType {
        mime: "image/x-portable-anymap",
        uti: "public.pnm",
        extensions: &["pnm", "pbm", "pgm", "ppm", "pam"],
    },
    SupportedType {
        mime: "image/x-portable-bitmap",
        uti: "public.pnm",
        extensions: &["pbm"],
    },
    SupportedType {
        mime: "image/x-portable-graymap",
        uti: "public.pnm",
        extensions: &["pgm"],
    },
    SupportedType {
        mime: "image/x-portable-pixmap",
        uti: "public.pnm",
        extensions: &["ppm"],
    },
    SupportedType {
        mime: "image/x-portable-arbitrarymap",
        uti: "public.pnm",
        extensions: &["pam"],
    },
    SupportedType {
        mime: "image/vnd.wap.wbmp",
        uti: "public.wbmp",
        extensions: &["wbmp"],
    },
    SupportedType {
        mime: "image/x-xbitmap",
        uti: "public.xbm",
        extensions: &["xbm"],
    },
    SupportedType {
        mime: "image/x-xpixmap",
        uti: "public.xpm",
        extensions: &["xpm"],
    },
    SupportedType {
        mime: "image/x-pcx",
        uti: "public.pcx",
        extensions: &["pcx"],
    },
    SupportedType {
        mime: "image/sgi",
        uti: "public.sgi-image",
        extensions: &["sgi", "bw", "rgb", "rgba"],
    },
    SupportedType {
        mime: "image/gif",
        uti: "com.compuserve.gif",
        extensions: &["gif"],
    },
    SupportedType {
        mime: "image/webp",
        uti: "org.webmproject.webp",
        extensions: &["webp"],
    },
    SupportedType {
        mime: "image/avif",
        uti: "public.avif",
        extensions: &["avif"],
    },
    SupportedType {
        mime: "image/jxl",
        uti: "public.jpeg-xl",
        extensions: &["jxl"],
    },
    SupportedType {
        mime: "image/svg+xml",
        uti: "public.svg-image",
        extensions: &["svg"],
    },
    SupportedType {
        mime: "image/svg+xml-compressed",
        uti: "public.svgz",
        extensions: &["svgz"],
    },
    SupportedType {
        mime: "image/vnd.adobe.photoshop",
        uti: "com.adobe.photoshop-image",
        extensions: &["psd"],
    },
    SupportedType {
        mime: "image/x-krita",
        uti: "org.krita.kra",
        extensions: &["kra"],
    },
    SupportedType {
        mime: "image/openraster",
        uti: "org.openraster.ora",
        extensions: &["ora"],
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn associations_match_packaging_and_dispatch() {
        let desktop = include_str!("../resources/glance.desktop");
        let mime_line = desktop
            .lines()
            .find_map(|line| line.strip_prefix("MimeType="))
            .unwrap();
        let packaged_mimes: BTreeSet<_> = mime_line.split(';').filter(|s| !s.is_empty()).collect();
        assert_eq!(packaged_mimes, TYPES.iter().map(|t| t.mime).collect());
        let script = include_str!("../scripts/package-macos.sh");
        let types = script
            .lines()
            .find_map(|line| line.strip_prefix("types='"))
            .unwrap()
            .strip_suffix('\'')
            .unwrap();
        let packaged: BTreeMap<_, BTreeSet<_>> = types
            .split_whitespace()
            .map(|entry| {
                let (uti, extensions) = entry.split_once(':').unwrap();
                (uti, extensions.split(',').collect())
            })
            .collect();
        let mut declared: BTreeMap<_, BTreeSet<_>> = BTreeMap::new();
        for entry in TYPES {
            declared
                .entry(entry.uti)
                .or_default()
                .extend(entry.extensions.iter().copied());
            for ext in entry.extensions {
                assert!(
                    crate::formats::format(std::path::Path::new(&format!("image.{ext}"))).is_some()
                );
            }
        }
        assert_eq!(packaged, declared);
    }
}
