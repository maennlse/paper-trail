fn main() {
    glib_build_tools::compile_resources(
        &["../.."],
        "resources/papertrail.gresource.xml",
        "papertrail.gresource",
    );
}
