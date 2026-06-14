use std::path::Path;

use iwork::{Document, DocumentKind, Error, PackageSupport, keynote, numbers, pages};

const NUMBERS_EXAMPLES: &[&str] = &[
    "examples/numbers/my_stocks.numbers",
    "examples/numbers/personal_budget.numbers",
    "examples/numbers/pivot_table.numbers",
    "examples/numbers/table_and_charts.numbers",
    "examples/numbers/more_types.numbers",
];

const PAGES_EXAMPLES: &[&str] = &[
    "examples/pages/modern_novel.pages",
    "examples/pages/term_paper.pages",
    "examples/pages/eternal_sunshine.pages",
];

const KEYNOTE_EXAMPLES: &[&str] = &[
    "examples/keynote/basic_white.key",
    "examples/keynote/blueprint.key",
    "examples/keynote/parchment.key",
    "examples/keynote/with_content.key",
];

#[test]
fn every_example_opens_and_exposes_core_metadata() -> Result<(), Error> {
    for path in NUMBERS_EXAMPLES
        .iter()
        .chain(PAGES_EXAMPLES.iter())
        .chain(KEYNOTE_EXAMPLES.iter())
    {
        let package = Document::open(path)?;
        let report = package.inspect((*path).to_owned())?;

        assert!(report.entry_count > 0, "{path} should not be empty");
        assert!(report.iwa_count > 0, "{path} should contain iwa payloads");
        assert_eq!(
            report.support,
            PackageSupport::SupportedDirectIndexEntries,
            "{path} should use the supported direct Index/ layout"
        );
        assert!(
            report.properties.document_uuid.is_some(),
            "{path} should expose a document uuid"
        );
        assert!(
            report.properties.file_format_version.is_some(),
            "{path} should expose a file format version"
        );
    }

    Ok(())
}

#[test]
fn examples_are_classified_by_extension() -> Result<(), Error> {
    for path in NUMBERS_EXAMPLES {
        let kind = Document::open(path)?.inspect((*path).to_owned())?.kind;
        assert_eq!(kind, DocumentKind::Numbers);
        assert_eq!(
            Path::new(path).extension().and_then(|value| value.to_str()),
            Some("numbers")
        );
        assert!(numbers::Document::open(path).is_ok());
    }

    for path in PAGES_EXAMPLES {
        let kind = Document::open(path)?.inspect((*path).to_owned())?.kind;
        assert_eq!(kind, DocumentKind::Pages);
        assert!(pages::Document::open(path).is_ok());
    }

    for path in KEYNOTE_EXAMPLES {
        let kind = Document::open(path)?.inspect((*path).to_owned())?.kind;
        assert_eq!(kind, DocumentKind::Keynote);
        assert!(keynote::Document::open(path).is_ok());
    }

    Ok(())
}

#[test]
fn pages_exposes_template_name() -> Result<(), Error> {
    let expected = [
        ("examples/pages/modern_novel.pages", "11B_Novel_Modern"),
        ("examples/pages/term_paper.pages", "04B_Term_Paper"),
    ];
    for (path, name) in expected {
        let body = pages::Document::open(path)?.document()?;
        assert_eq!(
            body.template_name(),
            Some(name),
            "{path}: expected template name {name:?}"
        );
    }
    Ok(())
}

#[test]
fn pages_exposes_media_descriptions() -> Result<(), Error> {
    let body = pages::Document::open("examples/pages/term_paper.pages")?.document()?;
    let media = body.media_descriptions();
    assert!(
        media.contains(&"blue sky viewed through a gap between canyon rocks".to_owned()),
        "expected canyon rocks description, got: {media:?}"
    );
    assert!(
        media.contains(&"sandstone buttes in Monument Valley under a partly cloudy sky".to_owned()),
        "expected Monument Valley description, got: {media:?}"
    );

    let body = pages::Document::open("examples/pages/modern_novel.pages")?.document()?;
    let media = body.media_descriptions();
    assert!(
        media.contains(&"evergreen trees under a starry night sky".to_owned()),
        "expected starry night sky description, got: {media:?}"
    );

    Ok(())
}

#[test]
fn keynote_exposes_theme_name() -> Result<(), Error> {
    let expected = [
        ("examples/keynote/basic_white.key", "21_BasicWhite"),
        ("examples/keynote/blueprint.key", "Blueprint"),
        ("examples/keynote/parchment.key", "Parchment"),
    ];
    for (path, name) in expected {
        let presentation = keynote::Document::open(path)?.presentation()?;
        assert_eq!(
            presentation.theme_name(),
            Some(name),
            "{path}: expected theme name {name:?}"
        );
    }
    Ok(())
}

#[test]
fn keynote_slide_template_distinction() -> Result<(), Error> {
    for path in KEYNOTE_EXAMPLES {
        let presentation = keynote::Document::open(path)?.presentation()?;
        let slides = presentation.slides();

        assert!(
            slides.iter().any(|s| !s.is_template()),
            "{path}: expected at least one non-template slide"
        );
        assert!(
            slides.iter().any(|s| s.is_template()),
            "{path}: expected at least one template slide"
        );
        for slide in slides {
            if slide.is_template() {
                assert!(
                    slide.path().contains("TemplateSlide"),
                    "{path}: template slide path should contain 'TemplateSlide', got {}",
                    slide.path()
                );
            } else {
                assert!(
                    !slide.path().contains("TemplateSlide"),
                    "{path}: non-template slide path should not contain 'TemplateSlide', got {}",
                    slide.path()
                );
            }
        }
    }
    Ok(())
}

#[test]
fn keynote_slides_expose_media_descriptions() -> Result<(), Error> {
    // blueprint.key real slides carry image alt-text; validate at least the
    // first real slide with media has the expected descriptions.
    let presentation = keynote::Document::open("examples/keynote/blueprint.key")?.presentation()?;
    let slide = presentation
        .slides()
        .iter()
        .find(|s| !s.is_template() && !s.media_descriptions().is_empty())
        .expect("blueprint.key should have a non-template slide with media");

    let media = slide.media_descriptions();
    assert!(
        media.contains(&"Modern living room with large windows".to_owned()),
        "expected 'Modern living room with large windows' in media descriptions, got: {media:?}"
    );
    assert!(
        media.contains(&"Wooden walkway with cabinets on either side and a staircase in the background".to_owned()),
        "expected walkway description in media descriptions, got: {media:?}"
    );

    // parchment.key: validate multi-image slide
    let presentation = keynote::Document::open("examples/keynote/parchment.key")?.presentation()?;
    let slide = presentation
        .slides()
        .iter()
        .find(|s| !s.is_template() && s.media_descriptions().len() >= 3)
        .expect("parchment.key should have a real slide with 3+ media descriptions");

    assert_eq!(
        slide.media_descriptions(),
        &[
            "Pyramids of Giza silhouetted against an orange sunset",
            "Close-up of a pyramid in Giza",
            "Sphinx in front of the pyramids of Giza with a clear blue sky in the background",
        ],
        "parchment.key multi-image slide descriptions mismatch"
    );

    Ok(())
}

#[test]
fn pages_exposes_text_fragments() -> Result<(), Error> {
    let body = pages::Document::open("examples/pages/eternal_sunshine.pages")?.document()?;

    assert_eq!(
        body.title(),
        Some("Eternal Shine"),
        "expected title to be 'Eternal Shine', got: {:?}",
        body.title()
    );

    assert!(!body.headings().is_empty(), "expected non-empty headings");

    let frags = body.text_fragments();
    assert!(
        frags.contains(&"Prologue".to_owned()),
        "expected 'Prologue' in text_fragments, got: {frags:?}"
    );
    assert!(
        frags.contains(&"Chapter 1".to_owned()),
        "expected 'Chapter 1' in text_fragments, got: {frags:?}"
    );
    assert!(
        frags.contains(&"About the Author".to_owned()),
        "expected 'About the Author' in text_fragments, got: {frags:?}"
    );

    Ok(())
}

#[test]
fn keynote_slides_expose_title() -> Result<(), Error> {
    let presentation =
        keynote::Document::open("examples/keynote/with_content.key")?.presentation()?;
    let slides: Vec<_> = presentation.slides().iter().filter(|s| !s.is_template()).collect();

    let title_slide = slides
        .iter()
        .find(|s| s.title() == Some("This is a presentation title"))
        .expect("with_content.key should have title slide");
    assert!(
        title_slide.text_fragments().contains(&"By Mr Author".to_owned()),
        "title slide text_fragments should include author line"
    );

    let content_slide = slides
        .iter()
        .find(|s| s.title() == Some("This is a slide title"))
        .expect("with_content.key should have a content slide with title");
    assert!(
        !content_slide.text_fragments().is_empty(),
        "content slide should have text_fragments"
    );

    // Image-only slides have no title
    assert!(
        slides.iter().any(|s| s.title().is_none()),
        "some slides should have no title"
    );

    Ok(())
}

#[test]
fn keynote_slides_expose_text_fragments() -> Result<(), Error> {
    let presentation =
        keynote::Document::open("examples/keynote/with_content.key")?.presentation()?;

    let title_slide = presentation
        .slides()
        .iter()
        .find(|s| !s.is_template() && s.text_fragments().contains(&"This is a presentation title".to_owned()))
        .expect("with_content.key should have a title slide");

    assert!(
        title_slide.text_fragments().contains(&"By Mr Author".to_owned()),
        "title slide should contain author line, got: {:?}",
        title_slide.text_fragments()
    );

    let content_slide = presentation
        .slides()
        .iter()
        .find(|s| !s.is_template() && s.text_fragments().contains(&"This is a slide title".to_owned()))
        .expect("with_content.key should have a slide with a title");

    assert!(
        !content_slide.text_fragments().is_empty(),
        "content slide should have text_fragments"
    );

    Ok(())
}
