use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SideEffect {
    Read,
    Draft,
    Write,
    Send,
    Purchase,
    Delete,
}

impl SideEffect {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Draft => "draft",
            Self::Write => "write",
            Self::Send => "send",
            Self::Purchase => "purchase",
            Self::Delete => "delete",
        }
    }

    pub fn requires_confirmation_by_default(&self) -> bool {
        matches!(self, Self::Send | Self::Purchase | Self::Delete)
    }

    pub fn requires_idempotency(&self) -> bool {
        !matches!(self, Self::Read | Self::Draft)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Auth {
    Public,
    User { scopes: Vec<String> },
    Admin { scopes: Vec<String> },
}

impl Auth {
    pub fn public() -> Self {
        Self::Public
    }

    pub fn user<const N: usize>(scopes: [&str; N]) -> Self {
        Self::User {
            scopes: scopes.iter().map(|scope| (*scope).to_string()).collect(),
        }
    }

    pub fn admin<const N: usize>(scopes: [&str; N]) -> Self {
        Self::Admin {
            scopes: scopes.iter().map(|scope| (*scope).to_string()).collect(),
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::User { .. } => "user",
            Self::Admin { .. } => "admin",
        }
    }

    fn scopes(&self) -> &[String] {
        match self {
            Self::Public => &[],
            Self::User { scopes } | Self::Admin { scopes } => scopes,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldKind {
    String,
    Number,
    Integer,
    Boolean,
    Object,
    Array,
}

impl FieldKind {
    fn json_schema_type(&self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Number => "number",
            Self::Integer => "integer",
            Self::Boolean => "boolean",
            Self::Object => "object",
            Self::Array => "array",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub kind: FieldKind,
    pub required: bool,
    pub description: Option<String>,
}

impl Field {
    pub fn new(name: impl Into<String>, kind: FieldKind) -> Self {
        Self {
            name: name.into(),
            kind,
            required: false,
            description: None,
        }
    }

    pub fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Schema {
    pub fields: Vec<Field>,
}

impl Schema {
    pub fn object<const N: usize>(fields: [Field; N]) -> Self {
        Self {
            fields: fields.into_iter().collect(),
        }
    }

    pub fn empty() -> Self {
        Self { fields: Vec::new() }
    }

    fn json_schema(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let inner = " ".repeat(indent + 2);
        let mut out = String::new();

        writeln!(out, "{{").ok();
        writeln!(out, "{inner}\"type\": \"object\",").ok();
        writeln!(out, "{inner}\"additionalProperties\": false,").ok();
        writeln!(out, "{inner}\"properties\": {{").ok();

        for (index, field) in self.fields.iter().enumerate() {
            let comma = if index + 1 == self.fields.len() {
                ""
            } else {
                ","
            };
            writeln!(out, "{inner}  \"{}\": {{", json_escape(&field.name)).ok();
            writeln!(
                out,
                "{inner}    \"type\": \"{}\"{}",
                field.kind.json_schema_type(),
                if field.description.is_some() { "," } else { "" }
            )
            .ok();
            if let Some(description) = &field.description {
                writeln!(
                    out,
                    "{inner}    \"description\": \"{}\"",
                    json_escape(description)
                )
                .ok();
            }
            writeln!(out, "{inner}  }}{comma}").ok();
        }

        writeln!(out, "{inner}}},").ok();
        write!(out, "{inner}\"required\": [").ok();
        let mut first = true;
        for field in self.fields.iter().filter(|field| field.required) {
            if !first {
                write!(out, ", ").ok();
            }
            first = false;
            write!(out, "\"{}\"", json_escape(&field.name)).ok();
        }
        writeln!(out, "]").ok();
        write!(out, "{pad}}}").ok();

        out
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resource {
    pub id: String,
    pub title: String,
    pub description: String,
    pub path: String,
    pub markdown_path: String,
    pub public: bool,
    pub tags: Vec<String>,
    pub last_modified: Option<String>,
}

impl Resource {
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        description: impl Into<String>,
        path: impl Into<String>,
        markdown_path: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            description: description.into(),
            path: path.into(),
            markdown_path: markdown_path.into(),
            public: true,
            tags: Vec::new(),
            last_modified: None,
        }
    }

    pub fn public(mut self, public: bool) -> Self {
        self.public = public;
        self
    }

    pub fn tags<const N: usize>(mut self, tags: [&str; N]) -> Self {
        self.tags = tags.iter().map(|tag| (*tag).to_string()).collect();
        self
    }

    pub fn last_modified(mut self, last_modified: impl Into<String>) -> Self {
        self.last_modified = Some(last_modified.into());
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    pub id: String,
    pub title: String,
    pub description: String,
    pub method: String,
    pub path: String,
    pub side_effect: SideEffect,
    pub auth: Auth,
    pub confirm: bool,
    pub dry_run: bool,
    pub idempotency_required: bool,
    pub input: Schema,
    pub output: Schema,
}

impl Action {
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        description: impl Into<String>,
        method: impl Into<String>,
        path: impl Into<String>,
        side_effect: SideEffect,
    ) -> Self {
        let confirm = side_effect.requires_confirmation_by_default();
        let idempotency_required = side_effect.requires_idempotency();
        Self {
            id: id.into(),
            title: title.into(),
            description: description.into(),
            method: method.into(),
            path: path.into(),
            side_effect,
            auth: Auth::Public,
            confirm,
            dry_run: false,
            idempotency_required,
            input: Schema::empty(),
            output: Schema::empty(),
        }
    }

    pub fn auth(mut self, auth: Auth) -> Self {
        self.auth = auth;
        self
    }

    pub fn confirm(mut self, confirm: bool) -> Self {
        self.confirm = confirm;
        self
    }

    pub fn dry_run(mut self, dry_run: bool) -> Self {
        self.dry_run = dry_run;
        self
    }

    pub fn input(mut self, input: Schema) -> Self {
        self.input = input;
        self
    }

    pub fn output(mut self, output: Schema) -> Self {
        self.output = output;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectApp {
    pub name: String,
    pub description: String,
    pub base_url: String,
    pub version: String,
    pub resources: Vec<Resource>,
    pub actions: Vec<Action>,
}

impl ConnectApp {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            base_url: trim_trailing_slash(base_url.into()),
            version: "0.1.0".to_string(),
            resources: Vec::new(),
            actions: Vec::new(),
        }
    }

    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    pub fn resource(mut self, resource: Resource) -> Self {
        self.resources.push(resource);
        self
    }

    pub fn action(mut self, action: Action) -> Self {
        self.actions.push(action);
        self
    }

    pub fn emit_bundle(&self) -> ConnectBundle {
        ConnectBundle {
            beater_manifest: self.beater_manifest_json(),
            agent_card: self.agent_card_json(),
            openapi: self.openapi_json(),
            mcp_catalog: self.mcp_catalog_json(),
            llms: self.llms_txt(),
            robots: self.robots_txt(),
            sitemap: self.sitemap_xml(),
        }
    }

    pub fn beater_manifest_json(&self) -> String {
        let mut out = String::new();
        writeln!(out, "{{").ok();
        writeln!(out, "  \"schema\": \"https://beater.dev/connect/v1\",").ok();
        writeln!(out, "  \"name\": \"{}\",", json_escape(&self.name)).ok();
        writeln!(
            out,
            "  \"description\": \"{}\",",
            json_escape(&self.description)
        )
        .ok();
        writeln!(out, "  \"version\": \"{}\",", json_escape(&self.version)).ok();
        writeln!(out, "  \"base_url\": \"{}\",", json_escape(&self.base_url)).ok();
        writeln!(out, "  \"endpoints\": {{").ok();
        writeln!(out, "    \"mcp\": \"/mcp\",").ok();
        writeln!(out, "    \"openapi\": \"/openapi.json\",").ok();
        writeln!(out, "    \"agent_card\": \"/.well-known/agent-card.json\",").ok();
        writeln!(out, "    \"llms\": \"/llms.txt\",").ok();
        writeln!(out, "    \"robots\": \"/robots.txt\",").ok();
        writeln!(out, "    \"sitemap\": \"/sitemap.xml\"").ok();
        writeln!(out, "  }},").ok();
        writeln!(
            out,
            "  \"capabilities\": [\"resources\", \"actions\", \"openapi\", \"mcp\", \"a2a\", \"crawl\", \"receipts\"],"
        )
        .ok();
        writeln!(out, "  \"auth\": {{").ok();
        writeln!(out, "    \"type\": \"oauth2\",").ok();
        writeln!(out, "    \"authorization_url\": \"/oauth/authorize\",").ok();
        writeln!(out, "    \"token_url\": \"/oauth/token\"").ok();
        writeln!(out, "  }},").ok();
        writeln!(out, "  \"resources\": {},", self.resources_json(2)).ok();
        writeln!(out, "  \"actions\": {}", self.actions_json(2)).ok();
        writeln!(out, "}}").ok();
        out
    }

    pub fn agent_card_json(&self) -> String {
        let mut out = String::new();
        writeln!(out, "{{").ok();
        writeln!(out, "  \"name\": \"{}\",", json_escape(&self.name)).ok();
        writeln!(
            out,
            "  \"description\": \"{}\",",
            json_escape(&self.description)
        )
        .ok();
        writeln!(out, "  \"url\": \"{}\",", json_escape(&self.base_url)).ok();
        writeln!(out, "  \"version\": \"{}\",", json_escape(&self.version)).ok();
        writeln!(out, "  \"preferred_transport\": \"mcp\",").ok();
        writeln!(out, "  \"interfaces\": [").ok();
        writeln!(out, "    {{ \"type\": \"mcp\", \"url\": \"/mcp\" }},").ok();
        writeln!(
            out,
            "    {{ \"type\": \"openapi\", \"url\": \"/openapi.json\" }}"
        )
        .ok();
        writeln!(out, "  ],").ok();
        writeln!(out, "  \"capabilities\": {{").ok();
        writeln!(out, "    \"streaming\": false,").ok();
        writeln!(out, "    \"push_notifications\": false,").ok();
        writeln!(out, "    \"state_transition_history\": true").ok();
        writeln!(out, "  }},").ok();
        writeln!(out, "  \"skills\": [").ok();
        let mut rows = Vec::new();
        for resource in &self.resources {
            rows.push(format!(
                "    {{ \"id\": \"read_{}\", \"name\": \"Read {}\", \"description\": \"{}\", \"tags\": [\"resource\"] }}",
                json_escape(&resource.id),
                json_escape(&resource.title),
                json_escape(&resource.description)
            ));
        }
        for action in &self.actions {
            rows.push(format!(
                "    {{ \"id\": \"{}\", \"name\": \"{}\", \"description\": \"{}\", \"tags\": [\"action\", \"{}\"] }}",
                json_escape(&action.id),
                json_escape(&action.title),
                json_escape(&action.description),
                action.side_effect.as_str()
            ));
        }
        writeln!(out, "{}", rows.join(",\n")).ok();
        writeln!(out, "  ],").ok();
        writeln!(out, "  \"security_schemes\": {{").ok();
        writeln!(out, "    \"oauth2\": {{").ok();
        writeln!(out, "      \"type\": \"oauth2\",").ok();
        writeln!(out, "      \"authorization_url\": \"/oauth/authorize\",").ok();
        writeln!(out, "      \"token_url\": \"/oauth/token\"").ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "  }}").ok();
        writeln!(out, "}}").ok();
        out
    }

    pub fn openapi_json(&self) -> String {
        let mut out = String::new();
        writeln!(out, "{{").ok();
        writeln!(out, "  \"openapi\": \"3.1.0\",").ok();
        writeln!(out, "  \"info\": {{").ok();
        writeln!(out, "    \"title\": \"{}\",", json_escape(&self.name)).ok();
        writeln!(
            out,
            "    \"description\": \"{}\",",
            json_escape(&self.description)
        )
        .ok();
        writeln!(out, "    \"version\": \"{}\"", json_escape(&self.version)).ok();
        writeln!(out, "  }},").ok();
        writeln!(
            out,
            "  \"servers\": [{{ \"url\": \"{}\" }}],",
            json_escape(&self.base_url)
        )
        .ok();
        writeln!(out, "  \"paths\": {{").ok();

        let mut path_rows = Vec::new();
        for resource in &self.resources {
            path_rows.push(self.resource_path_json(resource));
        }
        for action in &self.actions {
            path_rows.push(self.action_path_json(action));
        }
        writeln!(out, "{}", path_rows.join(",\n")).ok();
        writeln!(out, "  }},").ok();
        writeln!(out, "  \"components\": {{").ok();
        writeln!(out, "    \"securitySchemes\": {{").ok();
        writeln!(out, "      \"oauth2\": {{").ok();
        writeln!(out, "        \"type\": \"oauth2\",").ok();
        writeln!(out, "        \"flows\": {{").ok();
        writeln!(out, "          \"authorizationCode\": {{").ok();
        writeln!(
            out,
            "            \"authorizationUrl\": \"/oauth/authorize\","
        )
        .ok();
        writeln!(out, "            \"tokenUrl\": \"/oauth/token\",").ok();
        writeln!(out, "            \"scopes\": {}", self.scopes_json(12)).ok();
        writeln!(out, "          }}").ok();
        writeln!(out, "        }}").ok();
        writeln!(out, "      }}").ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "  }}").ok();
        writeln!(out, "}}").ok();
        out
    }

    pub fn mcp_catalog_json(&self) -> String {
        let mut out = String::new();
        writeln!(out, "{{").ok();
        writeln!(out, "  \"protocol\": \"mcp\",").ok();
        writeln!(out, "  \"server\": \"{}\",", json_escape(&self.name)).ok();
        writeln!(out, "  \"resources\": [").ok();
        let mut resource_rows = Vec::new();
        for resource in &self.resources {
            resource_rows.push(format!(
                "    {{ \"uri\": \"beater://resource/{}\", \"name\": \"{}\", \"description\": \"{}\", \"mimeType\": \"text/markdown\", \"href\": \"{}\" }}",
                json_escape(&resource.id),
                json_escape(&resource.title),
                json_escape(&resource.description),
                json_escape(&resource.markdown_path)
            ));
        }
        writeln!(out, "{}", resource_rows.join(",\n")).ok();
        writeln!(out, "  ],").ok();
        writeln!(out, "  \"tools\": [").ok();
        let mut tool_rows = Vec::new();
        for action in &self.actions {
            tool_rows.push(format!(
                "    {{ \"name\": \"{}\", \"description\": \"{}\", \"sideEffect\": \"{}\", \"confirm\": {}, \"dryRun\": {}, \"idempotencyRequired\": {}, \"auth\": {} , \"inputSchema\": {} }}",
                json_escape(&action.id),
                json_escape(&action.description),
                action.side_effect.as_str(),
                action.confirm,
                action.dry_run,
                action.idempotency_required,
                auth_json(&action.auth, 4),
                action.input.json_schema(4)
            ));
        }
        writeln!(out, "{}", tool_rows.join(",\n")).ok();
        writeln!(out, "  ],").ok();
        writeln!(out, "  \"prompts\": [").ok();
        writeln!(
            out,
            "    {{ \"name\": \"explore_site\", \"description\": \"Read public resources before calling actions.\" }},"
        )
        .ok();
        writeln!(
            out,
            "    {{ \"name\": \"preview_action\", \"description\": \"Use dry-run before mutating user state.\" }}"
        )
        .ok();
        writeln!(out, "  ]").ok();
        writeln!(out, "}}").ok();
        out
    }

    pub fn llms_txt(&self) -> String {
        let mut out = String::new();
        writeln!(out, "# {}", self.name).ok();
        writeln!(out).ok();
        writeln!(out, "{}", self.description).ok();
        writeln!(out).ok();
        writeln!(out, "Base URL: {}", self.base_url).ok();
        writeln!(out).ok();
        writeln!(out, "## Discovery").ok();
        writeln!(out).ok();
        writeln!(
            out,
            "- Beater manifest: {}/.well-known/beater.json",
            self.base_url
        )
        .ok();
        writeln!(
            out,
            "- Agent card: {}/.well-known/agent-card.json",
            self.base_url
        )
        .ok();
        writeln!(out, "- OpenAPI: {}/openapi.json", self.base_url).ok();
        writeln!(out, "- MCP: {}/mcp", self.base_url).ok();
        writeln!(out, "- Sitemap: {}/sitemap.xml", self.base_url).ok();
        writeln!(out).ok();
        writeln!(out, "## Resources").ok();
        writeln!(out).ok();
        for resource in &self.resources {
            writeln!(
                out,
                "- [{}]({}{}): {}",
                resource.title, self.base_url, resource.markdown_path, resource.description
            )
            .ok();
        }
        writeln!(out).ok();
        writeln!(out, "## Actions").ok();
        writeln!(out).ok();
        for action in &self.actions {
            writeln!(
                out,
                "- `{}`: {} Side effect: `{}`. Auth: `{}`. Confirm: `{}`. Dry run: `{}`.",
                action.id,
                action.description,
                action.side_effect.as_str(),
                action.auth.kind(),
                action.confirm,
                action.dry_run
            )
            .ok();
        }
        out
    }

    pub fn robots_txt(&self) -> String {
        format!(
            "User-agent: *\nAllow: /\n\nSitemap: {}/sitemap.xml\n\n# Agent discovery\n# llms: {}/llms.txt\n# beater: {}/.well-known/beater.json\n",
            self.base_url, self.base_url, self.base_url
        )
    }

    pub fn sitemap_xml(&self) -> String {
        let mut out = String::new();
        writeln!(out, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>").ok();
        writeln!(
            out,
            "<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">"
        )
        .ok();
        for resource in self.resources.iter().filter(|resource| resource.public) {
            writeln!(out, "  <url>").ok();
            writeln!(
                out,
                "    <loc>{}{}</loc>",
                xml_escape(&self.base_url),
                xml_escape(&resource.path)
            )
            .ok();
            if let Some(last_modified) = &resource.last_modified {
                writeln!(out, "    <lastmod>{}</lastmod>", xml_escape(last_modified)).ok();
            }
            writeln!(out, "  </url>").ok();
            writeln!(out, "  <url>").ok();
            writeln!(
                out,
                "    <loc>{}{}</loc>",
                xml_escape(&self.base_url),
                xml_escape(&resource.markdown_path)
            )
            .ok();
            if let Some(last_modified) = &resource.last_modified {
                writeln!(out, "    <lastmod>{}</lastmod>", xml_escape(last_modified)).ok();
            }
            writeln!(out, "  </url>").ok();
        }
        writeln!(out, "</urlset>").ok();
        out
    }

    fn resources_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let inner = " ".repeat(indent + 2);
        let mut out = String::new();
        writeln!(out, "[").ok();
        for (index, resource) in self.resources.iter().enumerate() {
            let comma = if index + 1 == self.resources.len() {
                ""
            } else {
                ","
            };
            writeln!(out, "{inner}{{").ok();
            writeln!(out, "{inner}  \"id\": \"{}\",", json_escape(&resource.id)).ok();
            writeln!(
                out,
                "{inner}  \"title\": \"{}\",",
                json_escape(&resource.title)
            )
            .ok();
            writeln!(
                out,
                "{inner}  \"description\": \"{}\",",
                json_escape(&resource.description)
            )
            .ok();
            writeln!(
                out,
                "{inner}  \"path\": \"{}\",",
                json_escape(&resource.path)
            )
            .ok();
            writeln!(
                out,
                "{inner}  \"markdown_path\": \"{}\",",
                json_escape(&resource.markdown_path)
            )
            .ok();
            writeln!(out, "{inner}  \"public\": {}", resource.public).ok();
            writeln!(out, "{inner}}}{comma}").ok();
        }
        write!(out, "{pad}]").ok();
        out
    }

    fn actions_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let inner = " ".repeat(indent + 2);
        let mut out = String::new();
        writeln!(out, "[").ok();
        for (index, action) in self.actions.iter().enumerate() {
            let comma = if index + 1 == self.actions.len() {
                ""
            } else {
                ","
            };
            writeln!(out, "{inner}{{").ok();
            writeln!(out, "{inner}  \"id\": \"{}\",", json_escape(&action.id)).ok();
            writeln!(
                out,
                "{inner}  \"title\": \"{}\",",
                json_escape(&action.title)
            )
            .ok();
            writeln!(
                out,
                "{inner}  \"description\": \"{}\",",
                json_escape(&action.description)
            )
            .ok();
            writeln!(
                out,
                "{inner}  \"method\": \"{}\",",
                json_escape(&action.method)
            )
            .ok();
            writeln!(out, "{inner}  \"path\": \"{}\",", json_escape(&action.path)).ok();
            writeln!(
                out,
                "{inner}  \"side_effect\": \"{}\",",
                action.side_effect.as_str()
            )
            .ok();
            writeln!(out, "{inner}  \"confirm\": {},", action.confirm).ok();
            writeln!(out, "{inner}  \"dry_run\": {},", action.dry_run).ok();
            writeln!(
                out,
                "{inner}  \"idempotency_required\": {},",
                action.idempotency_required
            )
            .ok();
            writeln!(
                out,
                "{inner}  \"auth\": {}",
                auth_json(&action.auth, indent + 2)
            )
            .ok();
            writeln!(out, "{inner}}}{comma}").ok();
        }
        write!(out, "{pad}]").ok();
        out
    }

    fn resource_path_json(&self, resource: &Resource) -> String {
        format!(
            "    \"{}\": {{\n      \"get\": {{\n        \"operationId\": \"read_{}\",\n        \"summary\": \"Read {}\",\n        \"description\": \"{}\",\n        \"responses\": {{\n          \"200\": {{ \"description\": \"Resource content\" }}\n        }}\n      }}\n    }}",
            json_escape(&resource.path),
            json_escape(&resource.id),
            json_escape(&resource.title),
            json_escape(&resource.description)
        )
    }

    fn action_path_json(&self, action: &Action) -> String {
        let method = action.method.to_ascii_lowercase();
        let security = match action.auth {
            Auth::Public => "[]".to_string(),
            _ => format!(
                "[{{ \"oauth2\": {} }}]",
                string_array_json(action.auth.scopes())
            ),
        };
        let idempotency_header = if action.idempotency_required {
            "\n        \"parameters\": [\n          {\n            \"name\": \"Idempotency-Key\",\n            \"in\": \"header\",\n            \"required\": true,\n            \"schema\": { \"type\": \"string\" }\n          }\n        ],"
        } else {
            ""
        };
        format!(
            "    \"{}\": {{\n      \"{}\": {{\n        \"operationId\": \"{}\",\n        \"summary\": \"{}\",\n        \"description\": \"{}\",\n        \"security\": {},{}\n        \"x-beater-connect\": {{\n          \"sideEffect\": \"{}\",\n          \"confirm\": {},\n          \"dryRun\": {},\n          \"idempotencyRequired\": {}\n        }},\n        \"requestBody\": {{\n          \"required\": true,\n          \"content\": {{\n            \"application/json\": {{\n              \"schema\": {}\n            }}\n          }}\n        }},\n        \"responses\": {{\n          \"200\": {{ \"description\": \"Action result\" }}\n        }}\n      }}\n    }}",
            json_escape(&action.path),
            json_escape(&method),
            json_escape(&action.id),
            json_escape(&action.title),
            json_escape(&action.description),
            security,
            idempotency_header,
            action.side_effect.as_str(),
            action.confirm,
            action.dry_run,
            action.idempotency_required,
            action.input.json_schema(14)
        )
    }

    fn scopes_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let inner = " ".repeat(indent + 2);
        let mut scopes = Vec::new();
        for action in &self.actions {
            for scope in action.auth.scopes() {
                if !scopes.contains(scope) {
                    scopes.push(scope.to_string());
                }
            }
        }
        let mut out = String::new();
        writeln!(out, "{{").ok();
        for (index, scope) in scopes.iter().enumerate() {
            let comma = if index + 1 == scopes.len() { "" } else { "," };
            writeln!(
                out,
                "{inner}\"{}\": \"{}\"{comma}",
                json_escape(scope),
                json_escape(&format!("Access scope {scope}"))
            )
            .ok();
        }
        write!(out, "{pad}}}").ok();
        out
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectBundle {
    pub beater_manifest: String,
    pub agent_card: String,
    pub openapi: String,
    pub mcp_catalog: String,
    pub llms: String,
    pub robots: String,
    pub sitemap: String,
}

pub fn demo_app() -> ConnectApp {
    ConnectApp::new(
        "Acme Store",
        "Product catalog and demo booking for AI agents.",
        "https://acme.example",
    )
    .resource(
        Resource::new(
            "products",
            "Products",
            "Browse public product information.",
            "/products",
            "/products.md",
        )
        .tags(["catalog"])
        .last_modified("2026-07-02"),
    )
    .resource(
        Resource::new(
            "support",
            "Support docs",
            "Read troubleshooting and onboarding documentation.",
            "/support",
            "/support.md",
        )
        .tags(["docs"])
        .last_modified("2026-07-02"),
    )
    .action(
        Action::new(
            "search_products",
            "Search products",
            "Search the public product catalog.",
            "POST",
            "/agent/actions/search-products",
            SideEffect::Read,
        )
        .input(Schema::object([Field::new("query", FieldKind::String)
            .required(true)
            .description("Search query.")]))
        .output(Schema::object([Field::new("results", FieldKind::Array)])),
    )
    .action(
        Action::new(
            "book_demo",
            "Book demo",
            "Schedule a product demo for the signed-in user.",
            "POST",
            "/agent/actions/book-demo",
            SideEffect::Write,
        )
        .auth(Auth::user(["demo:book"]))
        .confirm(true)
        .dry_run(true)
        .input(Schema::object([
            Field::new("email", FieldKind::String)
                .required(true)
                .description("User email address."),
            Field::new("time", FieldKind::String)
                .required(true)
                .description("Requested ISO-8601 appointment time."),
        ]))
        .output(Schema::object([
            Field::new("receipt_id", FieldKind::String).required(true),
            Field::new("status", FieldKind::String).required(true),
        ])),
    )
}

fn auth_json(auth: &Auth, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let inner = " ".repeat(indent + 2);
    let mut out = String::new();
    writeln!(out, "{{").ok();
    writeln!(out, "{inner}\"type\": \"{}\",", auth.kind()).ok();
    writeln!(
        out,
        "{inner}\"scopes\": {}",
        string_array_json(auth.scopes())
    )
    .ok();
    write!(out, "{pad}}}").ok();
    out
}

fn string_array_json(values: &[String]) -> String {
    let body = values
        .iter()
        .map(|value| format!("\"{}\"", json_escape(value)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{body}]")
}

fn json_escape(value: &str) -> String {
    let mut out = String::new();
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            character if character.is_control() => {
                write!(out, "\\u{:04x}", character as u32).ok();
            }
            character => out.push(character),
        }
    }
    out
}

fn xml_escape(value: &str) -> String {
    let mut out = String::new();
    for character in value.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            character => out.push(character),
        }
    }
    out
}

fn trim_trailing_slash(mut value: String) -> String {
    while value.ends_with('/') {
        value.pop();
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_effect_defaults_are_conservative() {
        assert!(!SideEffect::Read.requires_confirmation_by_default());
        assert!(!SideEffect::Draft.requires_confirmation_by_default());
        assert!(!SideEffect::Write.requires_confirmation_by_default());
        assert!(SideEffect::Send.requires_confirmation_by_default());
        assert!(SideEffect::Purchase.requires_confirmation_by_default());
        assert!(SideEffect::Delete.requires_confirmation_by_default());

        assert!(!SideEffect::Read.requires_idempotency());
        assert!(!SideEffect::Draft.requires_idempotency());
        assert!(SideEffect::Write.requires_idempotency());
    }

    #[test]
    fn generated_surfaces_include_the_same_actions() {
        let bundle = demo_app().emit_bundle();
        for action in ["search_products", "book_demo"] {
            assert!(bundle.beater_manifest.contains(action));
            assert!(bundle.agent_card.contains(action));
            assert!(bundle.openapi.contains(action));
            assert!(bundle.mcp_catalog.contains(action));
            assert!(bundle.llms.contains(action));
        }
    }

    #[test]
    fn mutating_actions_get_idempotency_header_in_openapi() {
        let openapi = demo_app().openapi_json();
        assert!(openapi.contains("\"Idempotency-Key\""));
        assert!(openapi.contains("\"idempotencyRequired\": true"));
    }

    #[test]
    fn private_resources_are_not_added_to_sitemap() {
        let app = ConnectApp::new("Private", "Private app", "https://example.com").resource(
            Resource::new(
                "orders",
                "Orders",
                "Private orders.",
                "/orders",
                "/orders.md",
            )
            .public(false),
        );

        let sitemap = app.sitemap_xml();
        assert!(!sitemap.contains("/orders"));
    }

    #[test]
    fn xml_values_are_escaped() {
        let app = ConnectApp::new("Escaped", "Escaped app", "https://example.com?x=1&y=2")
            .resource(Resource::new("docs", "Docs", "Docs.", "/a&b", "/a&b.md"));

        let sitemap = app.sitemap_xml();
        assert!(sitemap.contains("https://example.com?x=1&amp;y=2/a&amp;b"));
    }
}
