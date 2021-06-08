#[macro_use]
extern crate lazy_static;

use std::process::exit;

use comfy_table::presets::{UTF8_FULL, UTF8_HORIZONTAL_BORDERS_ONLY};
use comfy_table::{ContentArrangement, Table};
use http::download;
use indicatif::{ProgressBar, ProgressStyle};

mod cli;
mod epub;
mod errors;
mod extractor;
/// This module is responsible for async HTTP calls for downloading
/// the HTML content and images
mod http;
mod logs;
mod moz_readability;

use cli::AppConfig;
use epub::generate_epubs;
use logs::display_summary;

fn main() {
    let app_config = match cli::AppConfig::init_with_cli() {
        Ok(app_config) => app_config,
        Err(err) => {
            eprintln!("{}", err);
            exit(1);
        }
    };

    if !app_config.urls.is_empty() {
        run(app_config);
    }
}

fn run(app_config: AppConfig) {
    let mut errors = Vec::new();
    let mut partial_downloads = Vec::new();
    let bar = if app_config.can_disable_progress_bar {
        ProgressBar::hidden()
    } else {
        let enabled_bar = ProgressBar::new(app_config.urls.len() as u64);
        let style = ProgressStyle::default_bar().template(
            "{spinner:.cyan} [{elapsed_precise}] {bar:40.white} {:>8} link {pos}/{len:7} {msg:.yellow/white}",
        );
        enabled_bar.set_style(style);
        enabled_bar.enable_steady_tick(500);
        enabled_bar
    };
    let articles = download(&app_config, &bar, &mut partial_downloads, &mut errors);
    bar.finish_with_message("Downloaded articles");

    let mut succesful_articles_table = Table::new();
    succesful_articles_table
        .load_preset(UTF8_FULL)
        .load_preset(UTF8_HORIZONTAL_BORDERS_ONLY)
        .set_content_arrangement(ContentArrangement::Dynamic);
    match generate_epubs(articles, &app_config, &mut succesful_articles_table) {
        Ok(_) => (),
        Err(gen_epub_errors) => {
            errors.extend(gen_epub_errors);
        }
    };

    let has_errors = !errors.is_empty() || !partial_downloads.is_empty();
    display_summary(
        app_config.urls.len(),
        succesful_articles_table,
        partial_downloads,
        errors,
    );

    if app_config.is_logging_to_file {
        println!(
            "Log written to paperoni_{}.log\n",
            app_config.start_time.format("%Y-%m-%d_%H-%M-%S")
        );
    } else if has_errors && !app_config.is_logging_to_file {
        println!("\nRun paperoni with the --log-to-file flag to create a log file");
    }

    if has_errors {
        std::process::exit(1);
    }
}
