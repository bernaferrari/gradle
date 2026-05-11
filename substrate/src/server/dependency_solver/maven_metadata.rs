//! Maven metadata parsing for Gradle-compatible dependency resolution.

use quick_xml::events::Event;

/// Parsed `maven-metadata.xml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MavenMetadata {
    pub(crate) group_id: String,
    pub(crate) artifact_id: String,
    pub(crate) versioning: MavenVersioning,
}

/// Versioning section from `maven-metadata.xml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MavenVersioning {
    pub(crate) latest: Option<String>,
    pub(crate) release: Option<String>,
    pub(crate) last_updated: Option<String>,
    pub(crate) snapshot: Option<MavenSnapshot>,
    pub(crate) versions: Vec<String>,
}

/// Snapshot info from `maven-metadata.xml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MavenSnapshot {
    pub(crate) build_number: Option<String>,
    pub(crate) timestamp: Option<String>,
    pub(crate) local_copy: bool,
}

pub(crate) fn parse_maven_metadata(xml: &str) -> Result<MavenMetadata, String> {
    let mut reader = quick_xml::Reader::from_str(xml);
    reader.trim_text(true);

    let mut metadata = MavenMetadata {
        group_id: String::new(),
        artifact_id: String::new(),
        versioning: MavenVersioning {
            latest: None,
            release: None,
            last_updated: None,
            snapshot: None,
            versions: Vec::new(),
        },
    };

    let mut in_versions = false;
    let mut in_snapshot = false;
    let mut current_tag = String::new();
    let mut snapshot = MavenSnapshot {
        build_number: None,
        timestamp: None,
        local_copy: false,
    };

    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let name = e.name();
                match name.as_ref() {
                    b"versions" => in_versions = true,
                    b"snapshot" => {
                        in_snapshot = true;
                        snapshot = MavenSnapshot {
                            build_number: None,
                            timestamp: None,
                            local_copy: false,
                        };
                    }
                    _ => {
                        current_tag = std::str::from_utf8(name.local_name().as_ref())
                            .unwrap_or_default()
                            .to_string()
                    }
                }
            }
            Ok(Event::Empty(ref e)) => {
                let name = e.name();
                current_tag = std::str::from_utf8(name.local_name().as_ref())
                    .unwrap_or_default()
                    .to_string();
            }
            Ok(Event::Text(ref e)) => {
                let text = e.unescape().unwrap_or_default().to_string();
                if in_versions && current_tag == "version" {
                    metadata.versioning.versions.push(text);
                } else if in_snapshot {
                    match current_tag.as_str() {
                        "buildNumber" => snapshot.build_number = Some(text),
                        "timestamp" => snapshot.timestamp = Some(text),
                        "localCopy" => snapshot.local_copy = text == "true",
                        _ => {}
                    }
                } else {
                    match current_tag.as_str() {
                        "groupId" => metadata.group_id = text,
                        "artifactId" => metadata.artifact_id = text,
                        "latest" => metadata.versioning.latest = Some(text),
                        "release" => metadata.versioning.release = Some(text),
                        "lastUpdated" => metadata.versioning.last_updated = Some(text),
                        _ => {}
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                let name = e.name();
                match name.as_ref() {
                    b"versions" => in_versions = false,
                    b"snapshot" => {
                        in_snapshot = false;
                        metadata.versioning.snapshot = Some(snapshot.clone());
                    }
                    _ => {}
                }
                current_tag.clear();
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("XML parse error: {}", e)),
            _ => {}
        }
    }

    Ok(metadata)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versions_and_latest_release() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<metadata>
  <groupId>com.example</groupId>
  <artifactId>my-lib</artifactId>
  <versioning>
    <latest>3.0.0</latest>
    <release>2.5.0</release>
    <lastUpdated>20240101120000</lastUpdated>
    <versions>
      <version>1.0.0</version>
      <version>2.0.0</version>
      <version>2.5.0</version>
      <version>3.0.0</version>
    </versions>
  </versioning>
</metadata>"#;

        let meta = parse_maven_metadata(xml).unwrap();

        assert_eq!(meta.group_id, "com.example");
        assert_eq!(meta.artifact_id, "my-lib");
        assert_eq!(meta.versioning.latest.as_deref(), Some("3.0.0"));
        assert_eq!(meta.versioning.release.as_deref(), Some("2.5.0"));
        assert_eq!(
            meta.versioning.last_updated.as_deref(),
            Some("20240101120000")
        );
        assert_eq!(
            meta.versioning.versions,
            vec!["1.0.0", "2.0.0", "2.5.0", "3.0.0"]
        );
    }

    #[test]
    fn parses_snapshot_metadata() {
        let xml = r#"<?xml version="1.0"?>
<metadata>
  <groupId>com.example</groupId>
  <artifactId>my-lib</artifactId>
  <versioning>
    <snapshot>
      <timestamp>20240101120000</timestamp>
      <buildNumber>1</buildNumber>
      <localCopy>true</localCopy>
    </snapshot>
    <versions>
      <version>1.0.0-SNAPSHOT</version>
    </versions>
  </versioning>
</metadata>"#;

        let meta = parse_maven_metadata(xml).unwrap();
        let snapshot = meta.versioning.snapshot.unwrap();

        assert_eq!(snapshot.timestamp.as_deref(), Some("20240101120000"));
        assert_eq!(snapshot.build_number.as_deref(), Some("1"));
        assert!(snapshot.local_copy);
    }

    #[test]
    fn preserves_empty_metadata_as_empty_model() {
        let meta = parse_maven_metadata("<not-metadata/>").unwrap();

        assert!(meta.group_id.is_empty());
        assert!(meta.artifact_id.is_empty());
        assert!(meta.versioning.versions.is_empty());
    }
}
