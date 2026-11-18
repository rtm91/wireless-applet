// SPDX-License-Identifier: GPL-3.0-only
// Copyright 2026 Artem Wittermann

use std::sync::LazyLock;

use i18n_embed::{
    DefaultLocalizer, LanguageLoader, Localizer,
    fluent::{FluentLanguageLoader, fluent_language_loader},
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "i18n/"]
struct Localizations;

/// Holds the English strings until [`localize`] swaps in the user's language.
///
/// Loading the fallback eagerly means [`fl!`] works before `localize` runs,
/// which keeps the unit tests free of any setup.
pub static LANGUAGE_LOADER: LazyLock<FluentLanguageLoader> = LazyLock::new(|| {
    let loader: FluentLanguageLoader = fluent_language_loader!();

    loader
        .load_fallback_language(&Localizations)
        .expect("failed to load the fallback language");

    loader
});

#[macro_export]
macro_rules! fl {
    ($message_id:literal) => {{
        i18n_embed_fl::fl!($crate::localize::LANGUAGE_LOADER, $message_id)
    }};
}

/// Picks the language from the desktop environment's locale settings.
pub fn localize() {
    let localizer = DefaultLocalizer::new(&*LANGUAGE_LOADER, &Localizations);
    let requested = i18n_embed::DesktopLanguageRequester::requested_languages();

    if let Err(err) = localizer.select(&requested) {
        tracing::warn!(?err, "failed to load the requested language");
    }
}
