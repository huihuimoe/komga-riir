use komga_application::operational::{
    FilesystemBrowseError, FilesystemBrowsePort, FilesystemBrowseRequest,
    FilesystemDirectoryListing,
};

use crate::filesystem::browser;

#[derive(Clone, Default)]
pub struct FilesystemBrowseAccess;

impl FilesystemBrowsePort for FilesystemBrowseAccess {
    fn browse(
        &self,
        request: FilesystemBrowseRequest,
    ) -> Result<FilesystemDirectoryListing, FilesystemBrowseError> {
        browser::browse_directory(request)
    }
}
