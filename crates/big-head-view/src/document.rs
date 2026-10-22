//! Agent-authored documents with renderer-owned typography and escaped content.
use big_head_ai::ToolDefinition;
use serde::Deserialize;
use serde_json::{Value, json};

pub fn render_document_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "render_document".into(),
        description: "Publish a readable document in the View for substantial explanations, project plans, comparisons or learning notes. Use a stable id to update the same document. Supply plain text: the renderer owns layout and escaping. After successful publication give a short conversational summary and the returned route, not the entire document again. Short everyday replies stay in chat. Publication does not prove that the user opened or read the document.".into(),
        input_schema: json!({"type":"object","properties":{
            "id":{"type":"string","pattern":"^[a-z0-9][a-z0-9_-]{0,79}$"},
            "title":{"type":"string","maxLength":160},
            "summary":{"type":"string","maxLength":1200},
            "sections":{"type":"array","minItems":1,"maxItems":20,"items":{"type":"object","properties":{
                "heading":{"type":"string","maxLength":160},
                "body":{"type":"string","maxLength":10000},
                "bullets":{"type":"array","maxItems":20,"items":{"type":"string","maxLength":1000}}
            },"required":["heading","body"],"additionalProperties":false}}
        },"required":["id","title","summary","sections"],"additionalProperties":false}),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    id: String,
    title: String,
    summary: String,
    sections: Vec<Section>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Section {
    heading: String,
    body: String,
    #[serde(default)]
    bullets: Vec<String>,
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub fn normalize_document(input: &Value) -> Result<Value, String> {
    let doc: Document =
        serde_json::from_value(input.clone()).map_err(|e| format!("Document: {e}"))?;
    if doc.id.is_empty()
        || doc.id.len() > 80
        || !doc.id.as_bytes()[0].is_ascii_alphanumeric()
        || !doc
            .id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'_')
        || doc.title.trim().is_empty()
        || doc.title.chars().count() > 160
        || doc.summary.chars().count() > 1200
        || !(1..=20).contains(&doc.sections.len())
        || input.to_string().len() > 96000
    {
        return Err("Document requires a stable lowercase id, title (1..160 chars), summary (up to 1200 chars), 1..20 sections and at most 96 KB.".into());
    }
    let mut markup = format!(
        "<article class=\"bh-document\"><header><p class=\"bh-document-summary\">{}</p></header>",
        escape(&doc.summary)
    );
    for section in doc.sections {
        if section.heading.trim().is_empty()
            || section.heading.chars().count() > 160
            || section.body.chars().count() > 10000
            || section.bullets.len() > 20
            || section.bullets.iter().any(|s| s.chars().count() > 1000)
        {
            return Err("Document section exceeds heading/body/bullet limits".into());
        }
        markup.push_str(&format!("<section><h2>{}</h2>", escape(&section.heading)));
        for p in section.body.split("\n\n").filter(|s| !s.trim().is_empty()) {
            markup.push_str(&format!("<p>{}</p>", escape(p)));
        }
        if !section.bullets.is_empty() {
            markup.push_str("<ul>");
            for item in section.bullets {
                markup.push_str(&format!("<li>{}</li>", escape(&item)));
            }
            markup.push_str("</ul>");
        }
        markup.push_str("</section>");
    }
    markup.push_str("</article>");
    Ok(
        json!({"op":"spawn","id":format!("document-{}",doc.id),"surface":"html2d","kind":"html","title":doc.title,"position":[0,0,0],"content":{"kind":"html","markup":markup}}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn document_text_is_escaped_and_id_is_stable() {
        let input = json!({"id":"notes","title":"Notes","summary":"A & B","sections":[{"heading":"<script>x</script>","body":"<img onerror='x'>","bullets":["one"]}]});
        let doc = normalize_document(&input).unwrap();
        assert_eq!(doc["id"], "document-notes");
        let html = doc["content"]["markup"].as_str().unwrap();
        assert!(!html.contains("<script>"));
        assert!(!html.contains("<img"));
        assert!(html.contains("&amp;"));
        let mut invalid = input;
        invalid["id"] = json!("../bad");
        assert!(normalize_document(&invalid).is_err());
    }
}
