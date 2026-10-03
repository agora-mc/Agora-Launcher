use quick_xml::events::Event;
use quick_xml::reader::Reader;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MicrosoftGameConfig {
    pub identity_name: String,
    pub identity_version: Option<String>,
    pub default_display_name: Option<String>,
    pub main_package_dependency: Option<String>,
    pub executables: Vec<String>,
    pub has_executable_list: bool,
}

#[derive(Debug, Clone)]
pub struct ConfigParseError(pub String);

impl fmt::Display for ConfigParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ConfigParseError {}

pub fn parse_microsoft_game_config(
    xml_content: &str,
) -> Result<MicrosoftGameConfig, ConfigParseError> {
    let mut reader = Reader::from_str(xml_content);
    reader.config_mut().trim_text(true);

    let mut identity_name = String::new();
    let mut identity_version = None;
    let mut default_display_name = None;
    let mut main_package_dependency = None;
    let mut executables = Vec::new();
    let mut has_executable_list = false;

    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let local_name = e.local_name();
                let tag_str = String::from_utf8_lossy(local_name.as_ref());

                if tag_str.eq_ignore_ascii_case("Identity") {
                    for attr in e.attributes().flatten() {
                        let local = attr.key.local_name();
                        let key = String::from_utf8_lossy(local.as_ref());
                        let val = attr
                            .decode_and_unescape_value(reader.decoder())
                            .map(|v| v.into_owned())
                            .unwrap_or_default();
                        if key.eq_ignore_ascii_case("Name") {
                            identity_name = val;
                        } else if key.eq_ignore_ascii_case("Version") {
                            identity_version = Some(val);
                        }
                    }
                } else if tag_str.eq_ignore_ascii_case("ExecutableList") {
                    has_executable_list = true;
                } else if tag_str.eq_ignore_ascii_case("Executable") {
                    for attr in e.attributes().flatten() {
                        let local = attr.key.local_name();
                        let key = String::from_utf8_lossy(local.as_ref());
                        let val = attr
                            .decode_and_unescape_value(reader.decoder())
                            .map(|v| v.into_owned())
                            .unwrap_or_default();
                        if key.eq_ignore_ascii_case("Name") && !val.trim().is_empty() {
                            executables.push(val);
                        }
                    }
                } else if tag_str.eq_ignore_ascii_case("ShellVisuals") {
                    for attr in e.attributes().flatten() {
                        let local = attr.key.local_name();
                        let key = String::from_utf8_lossy(local.as_ref());
                        let val = attr
                            .decode_and_unescape_value(reader.decoder())
                            .map(|v| v.into_owned())
                            .unwrap_or_default();
                        if key.eq_ignore_ascii_case("DefaultDisplayName") && !val.trim().is_empty()
                        {
                            default_display_name = Some(val);
                        }
                    }
                } else if tag_str.eq_ignore_ascii_case("MainPackageDependency") {
                    for attr in e.attributes().flatten() {
                        let local = attr.key.local_name();
                        let key = String::from_utf8_lossy(local.as_ref());
                        let val = attr
                            .decode_and_unescape_value(reader.decoder())
                            .map(|v| v.into_owned())
                            .unwrap_or_default();
                        if key.eq_ignore_ascii_case("Name") && !val.trim().is_empty() {
                            main_package_dependency = Some(val);
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(err) => {
                return Err(ConfigParseError(format!("XML parse error: {err}")));
            }
            _ => {}
        }
        buf.clear();
    }

    if identity_name.trim().is_empty() {
        return Err(ConfigParseError("missing Identity Name".to_string()));
    }

    Ok(MicrosoftGameConfig {
        identity_name,
        identity_version,
        default_display_name,
        main_package_dependency,
        executables,
        has_executable_list,
    })
}
