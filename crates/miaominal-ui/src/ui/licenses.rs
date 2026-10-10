use anyhow::{Context, Result};

#[derive(Clone, Copy)]
pub(crate) enum LicenseDocument {
    Project,
    ThirdParty,
}

impl LicenseDocument {
    pub(crate) fn open(self) -> Result<()> {
        let (filename, contents) = match self {
            Self::Project => (
                "Miaominal-GPLv3.txt",
                include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../LICENSE")),
            ),
            Self::ThirdParty => (
                "Miaominal-Third-Party-Notices.txt",
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../assets/licenses/THIRD_PARTY_NOTICES.txt"
                )),
            ),
        };
        // Embedding the texts keeps every distribution, including standalone
        // executables and portable builds, independent of adjacent files.
        let directory = miaominal_paths::active_data_dir()?.join("licenses");
        std::fs::create_dir_all(&directory).context("create license document directory")?;
        let path = directory.join(filename);
        miaominal_paths::atomic_write(&path, contents)?;
        open::that(&path).context("open license document")
    }
}
