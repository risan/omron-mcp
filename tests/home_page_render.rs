//! The README renderer that build.rs uses for the public home page.

#[path = "../build/render.rs"]
mod render;

use render::render;

#[test]
fn raw_html_is_shown_as_text() {
    for markdown in [
        "<SCRIPT>alert(1)</SCRIPT>\n",
        "<img src=\"x\" onerror=\"alert(1)\">\n",
        "Text with <a href=\"/private\">an inline link</a> in it.\n",
        "<div onclick=\"alert(1)\">block</div>\n",
    ] {
        let html = render(markdown).to_ascii_lowercase();

        assert!(!html.contains("<script"), "{markdown} -> {html}");
        assert!(!html.contains("<img"), "{markdown} -> {html}");
        assert!(
            !html.contains("<a href=\"/private\""),
            "{markdown} -> {html}"
        );
        assert!(!html.contains("<div"), "{markdown} -> {html}");
        assert!(html.contains("&lt;"), "{markdown} -> {html}");
    }
}

#[test]
fn the_setup_guide_link_keeps_its_words() {
    let html = render("See the [setup guide](docs/setup.md).\n");

    assert!(html.contains("See the setup guide."), "{html}");
    assert!(!html.contains("docs/setup.md"), "{html}");
}

#[test]
#[should_panic(expected = "the page is served alone")]
fn a_relative_link_stops_the_build() {
    render("[report](.works/report.md)\n");
}

#[test]
fn an_https_image_is_kept() {
    let html = render("![logo](https://example.com/logo.png)\n");

    assert!(
        html.contains(r#"src="https://example.com/logo.png""#),
        "{html}"
    );
}

#[test]
fn an_image_that_is_not_https_stops_the_build() {
    for markdown in [
        "![x](javascript:alert(1))\n",
        "![x](data:image/png;base64,AAAA)\n",
        "![x](assets/favicon.ico)\n",
        "![x][ref]\n\n[ref]: &#106;avascript:alert(1)\n",
    ] {
        let rendered = std::panic::catch_unwind(|| render(markdown));

        assert!(rendered.is_err(), "{markdown} rendered");
    }
}
