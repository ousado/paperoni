use std::collections::HashMap;
use std::fs::File;

use comfy_table::{Attribute, Cell, CellAlignment, Color, ContentArrangement, Table};
use epub_builder::{EpubBuilder, EpubContent, TocElement, ZipLibrary};
use indicatif::{ProgressBar, ProgressStyle};
use kuchiki::NodeRef;
use log::{debug, info};

use crate::{
    cli::AppConfig,
    errors::PaperoniError,
    extractor::{self, Extractor},
};

pub fn generate_epubs(
    articles: Vec<Extractor>,
    app_config: &AppConfig,
    successful_articles_table: &mut Table,
) -> Result<(), Vec<PaperoniError>> {
    let bar = if app_config.can_disable_progress_bar() {
        ProgressBar::hidden()
    } else {
        let enabled_bar = ProgressBar::new(articles.len() as u64);
        let style = ProgressStyle::default_bar().template(
        "{spinner:.cyan} [{elapsed_precise}] {bar:40.white} {:>8} epub {pos}/{len:7} {msg:.green}",
    );
        enabled_bar.set_style(style);
        if !articles.is_empty() {
            enabled_bar.set_message("Generating epubs");
        }
        enabled_bar
    };

    let mut errors: Vec<PaperoniError> = Vec::new();

    match app_config.merged() {
        Some(name) => {
            successful_articles_table.set_header(vec![Cell::new("Table of Contents")
                .add_attribute(Attribute::Bold)
                .set_alignment(CellAlignment::Center)
                .fg(Color::Green)]);

            let mut epub = match EpubBuilder::new(match ZipLibrary::new() {
                Ok(zip_library) => zip_library,
                Err(err) => {
                    let mut paperoni_err: PaperoniError = err.into();
                    paperoni_err.set_article_source(name);
                    errors.push(paperoni_err);
                    return Err(errors);
                }
            }) {
                Ok(epub) => epub,
                Err(err) => {
                    let mut paperoni_err: PaperoniError = err.into();
                    paperoni_err.set_article_source(name);
                    errors.push(paperoni_err);
                    return Err(errors);
                }
            };
            debug!("Creating {:?}", name);
            epub.inline_toc();
            articles
                .iter()
                .enumerate()
                .fold(&mut epub, |epub, (idx, article)| {
                    let mut article_result = || -> Result<(), PaperoniError> {
                        let mut xhtml_buf = Vec::new();
                        extractor::serialize_to_xhtml(article.article(), &mut xhtml_buf)?;
                        let xhtml_str = std::str::from_utf8(&xhtml_buf)?;
                        let section_name = article.metadata().title();
                        let content_url = format!("article_{}.xhtml", idx);
                        let mut content = EpubContent::new(&content_url, xhtml_str.as_bytes())
                            .title(replace_escaped_characters(section_name));
                        let header_level_tocs =
                            get_header_level_toc_vec(&content_url, article.article());

                        for toc_element in header_level_tocs {
                            content = content.child(toc_element);
                        }

                        epub.metadata("title", replace_escaped_characters(name))?;
                        epub.add_content(content)?;
                        info!("Adding images for {:?}", name);
                        article.img_urls.iter().for_each(|img| {
                            // TODO: Add error handling and return errors as a vec
                            let mut file_path = std::env::temp_dir();
                            file_path.push(&img.0);

                            let img_buf = File::open(&file_path).expect("Can't read file");
                            epub.add_resource(
                                file_path.file_name().unwrap(),
                                img_buf,
                                img.1.as_ref().unwrap(),
                            )
                            .unwrap();
                        });
                        info!("Added images for {:?}", name);
                        Ok(())
                    };
                    if let Err(mut error) = article_result() {
                        error.set_article_source(&article.url);
                        errors.push(error);
                    }
                    bar.inc(1);
                    successful_articles_table.add_row(vec![article.metadata().title()]);
                    epub
                });
            let appendix = generate_appendix(articles.iter().collect());
            if let Err(err) = epub.add_content(
                EpubContent::new("appendix.xhtml", appendix.as_bytes())
                    .title(replace_escaped_characters("Article Sources")),
            ) {
                let mut paperoni_err: PaperoniError = err.into();
                paperoni_err.set_article_source(name);
                errors.push(paperoni_err);
                return Err(errors);
            }

            let mut out_file = File::create(&name).unwrap();
            match epub.generate(&mut out_file) {
                Ok(_) => (),
                Err(err) => {
                    let mut paperoni_err: PaperoniError = err.into();
                    paperoni_err.set_article_source(name);
                    errors.push(paperoni_err);
                    return Err(errors);
                }
            }

            bar.finish_with_message("Generated epub\n");
            debug!("Created {:?}", name);
            println!("Created {:?}", name);
        }
        None => {
            successful_articles_table
                .set_header(vec![Cell::new("Downloaded articles")
                    .add_attribute(Attribute::Bold)
                    .set_alignment(CellAlignment::Center)
                    .fg(Color::Green)])
                .set_content_arrangement(ContentArrangement::Dynamic);

            for article in &articles {
                let mut result = || -> Result<(), PaperoniError> {
                    let mut epub = EpubBuilder::new(ZipLibrary::new()?)?;
                    let file_name = format!(
                        "{}.epub",
                        article
                            .metadata()
                            .title()
                            .replace("/", " ")
                            .replace("\\", " ")
                    );
                    debug!("Creating {:?}", file_name);
                    let mut out_file = File::create(&file_name).unwrap();
                    let mut xhtml_buf = Vec::new();
                    extractor::serialize_to_xhtml(article.article(), &mut xhtml_buf)
                        .expect("Unable to serialize to xhtml");
                    let xhtml_str = std::str::from_utf8(&xhtml_buf).unwrap();
                    let header_level_tocs =
                        get_header_level_toc_vec("index.xhtml", article.article());

                    if let Some(author) = article.metadata().byline() {
                        epub.metadata("author", replace_escaped_characters(author))?;
                    }
                    let title = replace_escaped_characters(article.metadata().title());
                    epub.metadata("title", &title)?;

                    let mut content =
                        EpubContent::new("index.xhtml", xhtml_str.as_bytes()).title(title);

                    for toc_element in header_level_tocs {
                        content = content.child(toc_element);
                    }

                    epub.add_content(content)?;

                    for img in &article.img_urls {
                        let mut file_path = std::env::temp_dir();
                        file_path.push(&img.0);

                        let img_buf = File::open(&file_path).expect("Can't read file");
                        epub.add_resource(
                            file_path.file_name().unwrap(),
                            img_buf,
                            img.1.as_ref().unwrap(),
                        )?;
                    }
                    let appendix = generate_appendix(vec![&article]);
                    epub.add_content(
                        EpubContent::new("appendix.xhtml", appendix.as_bytes())
                            .title(replace_escaped_characters("Article Source")),
                    )?;
                    epub.generate(&mut out_file)?;
                    bar.inc(1);

                    successful_articles_table.add_row(vec![article.metadata().title()]);

                    debug!("Created {:?}", file_name);
                    Ok(())
                };
                if let Err(mut error) = result() {
                    error.set_article_source(&article.url);
                    errors.push(error);
                }
            }
            bar.finish_with_message("Generated epubs\n");
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Replaces characters that have to be escaped before adding to the epub's metadata
fn replace_escaped_characters(value: &str) -> String {
    value
        .replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
}

//TODO: The type signature of the argument should change as it requires that merged articles create an entirely new Vec of references
fn generate_appendix(articles: Vec<&Extractor>) -> String {
    let link_tags: String = articles
        .iter()
        .map(|article| {
            let article_name = if !article.metadata().title().is_empty() {
                article.metadata().title()
            } else {
                &article.url
            };
            format!(
                "<a href=\"{}\">{}</a><br></br>",
                replace_escaped_characters(&article.url),
                replace_escaped_characters(article_name)
            )
        })
        .collect();
    let template = format!(
        r#"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
    <head>
    </head>
    <body>
        <h2>Appendix</h2><h3>Article sources</h3>
        {}
    </body>
</html>"#,
        link_tags
    );
    template
}

/// Adds an id attribute to header elements and assigns a value based on
/// the hash of the text content. Headers with id attributes are not modified.
/// The headers here are known to have text because the grabbed article from
/// readability removes headers with no text.
fn generate_header_ids(root_node: &NodeRef) {
    let headers = root_node
        .select("h1, h2, h3, h4")
        .expect("Unable to create selector for headings");
    let headers_no_id = headers.filter(|node_data_ref| {
        let attrs = node_data_ref.attributes.borrow();
        !attrs.contains("id")
    });
    for header in headers_no_id {
        let mut attrs = header.attributes.borrow_mut();
        let text = header.text_contents();
        // The value of the id begins with an underscore because the hexadecimal
        // digest might start with a number which would make it an invalid id
        // when querying with selectors
        let value = format!("_{:x}", md5::compute(text));
        attrs.insert("id", value);
    }
}

/// Returns a vector of `TocElement` from a NodeRef used for adding to the Table of Contents for navigation
fn get_header_level_toc_vec(content_url: &str, article: &NodeRef) -> Vec<TocElement> {
    generate_header_ids(article);

    let mut headers_vec = Vec::new();

    let mut header_levels = HashMap::new();
    header_levels.insert("h1", 1);
    header_levels.insert("h2", 2);
    header_levels.insert("h3", 3);
    header_levels.insert("h4", 4);

    let headings = article
        .select("h1, h2, h3, h4")
        .expect("Unable to create selector for headings");

    let mut last_toc_elem_level: Option<i32> = None;

    for heading in headings {
        // TODO: Create a new function that adds an id attribute to heading tags before this function is called
        let elem_attrs = heading.attributes.borrow();
        let elem_name: &str = &heading.name.local;
        let elem_level = header_levels[elem_name];
        let id = elem_attrs.get("id").map(|val| val.to_string()).unwrap();
        let toc = TocElement::new(
            format!("{}#{}", content_url, id),
            replace_escaped_characters(&heading.text_contents()),
        );

        if let Some(last_elem_level) = last_toc_elem_level {
            if elem_level <= last_elem_level {
                last_toc_elem_level = Some(elem_level);
                headers_vec.push(toc);
            } else {
                match headers_vec.last_mut() {
                    Some(toc_elem) => *toc_elem = toc_elem.clone().child(toc),
                    _ => unreachable!(),
                }
            }
        } else {
            last_toc_elem_level = Some(elem_level);
            headers_vec.push(toc);
        }
    }
    headers_vec
}
#[cfg(test)]
mod test {
    use kuchiki::traits::*;

    use super::{generate_header_ids, get_header_level_toc_vec, replace_escaped_characters};

    #[test]
    fn test_replace_escaped_characters() {
        let mut value = "Lorem ipsum";
        assert_eq!(replace_escaped_characters(value), "Lorem ipsum");
        value = "Memory safe > memory unsafe";
        assert_eq!(
            replace_escaped_characters(value),
            "Memory safe &gt; memory unsafe"
        );
        value = "Author Name <author@mail.example>";
        assert_eq!(
            replace_escaped_characters(value),
            "Author Name &lt;author@mail.example&gt;"
        );
    }

    #[test]
    fn test_generate_header_ids() {
        let html_str = r#"
<!DOCTYPE html>
<html>
    <body>
        <h1>Heading 1</h1>
        <h2 id="heading-2">Heading 2</h2>
        <h2 id="heading-2-again">Heading 2 again</h2>
        <h4>Heading 4</h4>
        <h1>Heading 1 again</h1>
        <h3 class="heading">Heading 3</h3>
    </body>
</html>
        "#;
        let doc = kuchiki::parse_html().one(html_str);
        generate_header_ids(&doc);

        let mut headers = doc.select("h1, h2, h3, h4").unwrap();
        let all_headers_have_ids = headers.all(|node_data_ref| {
            let attrs = node_data_ref.attributes.borrow();
            if let Some(id) = attrs.get("id") {
                !id.trim().is_empty()
            } else {
                false
            }
        });
        assert_eq!(true, all_headers_have_ids);

        let selector = format!("h1#_{:x}", md5::compute("Heading 1"));
        assert_eq!(true, doc.select_first(&selector).is_ok());

        let selector = format!("h1#_{:x}", md5::compute("Heading 1 again"));
        assert_eq!(true, doc.select_first(&selector).is_ok());

        let selector = "h2#heading-2-again";
        assert_eq!(true, doc.select_first(selector).is_ok());
    }

    #[test]
    fn test_get_header_level_toc_vec() {
        // NOTE: Due to `TocElement` not implementing PartialEq, the tests here
        // will need to be manually written to cover for this
        let html_str = r#"
        <!DOCTYPE html>
        <html>
            <body>
                <p>Lorem ipsum</p>
            </body>
        </html>
        "#;
        let doc = kuchiki::parse_html().one(html_str);

        let toc_vec = get_header_level_toc_vec("index.xhtml", &doc);
        assert_eq!(0, toc_vec.len());

        let html_str = r#"
        <!DOCTYPE html>
        <html>
            <body>
                <h1 id="heading-1">Heading 1</h1>
                <p>Lorem ipsum</p>
                <div>
                    <h2 id="heading-2">Heading 2</h2>
                    <p>Lorem ipsum</p>
                    <p>Lorem ipsum</p>
                </div>
                <h3 id="subheading-3">Subheading 3</h2>
                <p>Lorem ipsum</p>
                <h1 id="heading-2">Second Heading 1</h2>
                <p>Lorem ipsum</p>
            </body>
        </html>
        "#;
        let doc = kuchiki::parse_html().one(html_str);

        let toc_vec = get_header_level_toc_vec("index.xhtml", &doc);
        assert_eq!(2, toc_vec.len());

        let first_h1_toc = toc_vec.first().unwrap();
        assert_eq!("Heading 1", first_h1_toc.title);
        assert_eq!(1, first_h1_toc.children.len());

        let h2_toc = first_h1_toc.children.first().unwrap();
        assert_eq!("Heading 2", h2_toc.title);
        assert_eq!(1, h2_toc.children.len());

        let h3_toc = h2_toc.children.first().unwrap();
        assert_eq!("Subheading 3", h3_toc.title);
        assert_eq!(0, h3_toc.children.len());

        let last_h1_toc = toc_vec.last().unwrap();
        assert_eq!("Second Heading 1", last_h1_toc.title);
        assert_eq!(0, last_h1_toc.children.len());

        let html_str = r#"
        <!DOCTYPE html>
        <html>
            <body>
                <h1 id="heading-1">Heading 1</h1>
                <p>Lorem ipsum</p>
                <div>
                    <h2 id="heading-2">Heading 2</h2>
                    <p>Lorem ipsum</p>
                    <p>Lorem ipsum</p>
                    <h3 id="subheading-3">Subheading 3</h2>
                    <p>Lorem ipsum</p>
                </div>
                <h2 id="heading-2">Heading 2</h2>
                <p>Lorem ipsum</p>
                <h4 id="subheading-4">Subheading 4</h4>
                <h2 id="conclusion">Conclusion</h2>
            </body>
        </html>
        "#;
        let doc = kuchiki::parse_html().one(html_str);

        let toc_vec = get_header_level_toc_vec("index.xhtml", &doc);
        assert_eq!(1, toc_vec.len());

        let h1_toc = toc_vec.first().unwrap();
        assert_eq!("Heading 1", h1_toc.title);
        assert_eq!(3, h1_toc.children.len());

        let first_h2_toc = h1_toc.children.first().unwrap();
        assert_eq!("Heading 2", first_h2_toc.title);
        assert_eq!(1, first_h2_toc.children.len());

        let h3_toc = first_h2_toc.children.first().unwrap();
        assert_eq!("Subheading 3", h3_toc.title);
        assert_eq!(0, h3_toc.children.len());
    }
}
