use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd, html};

const PRIVATE_LINK: &str = "docs/setup.md";

/// Renders the README for the public home page. Raw HTML is shown as text, so an edit to the
/// README cannot put a script or an event handler on the page; a link to the setup guide keeps
/// its words but loses the link, because the page is served alone and a relative link breaks.
pub fn render(markdown: &str) -> String {
    let mut inside_private_link = false;
    let events =
        Parser::new_ext(markdown, Options::ENABLE_TABLES).filter_map(|event| match event {
            Event::Start(Tag::Link { ref dest_url, .. }) => {
                if &**dest_url == PRIVATE_LINK {
                    inside_private_link = true;

                    return None;
                }

                assert!(
                    dest_url.starts_with("https://") || dest_url.starts_with('#'),
                    "the home page cannot link to {dest_url}: the page is served alone, so use an https link"
                );

                Some(event)
            }
            Event::Start(Tag::Image { ref dest_url, .. }) => {
                assert!(
                    dest_url.starts_with("https://"),
                    "the home page cannot show the image {dest_url}: only https:// images"
                );

                Some(event)
            }
            Event::End(TagEnd::Link) if inside_private_link => {
                inside_private_link = false;

                None
            }
            Event::Html(raw) | Event::InlineHtml(raw) => Some(Event::Text(raw)),
            other => Some(other),
        });
    let mut html = String::new();

    html::push_html(&mut html, events);

    html
}
