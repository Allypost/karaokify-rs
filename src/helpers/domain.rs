use url::Url;

pub struct DomainParser;
impl DomainParser {
    pub fn get_domain(url: &Url) -> Option<addr::domain::Name<'_>> {
        url.domain().and_then(|x| addr::parse_domain_name(x).ok())
    }

    /// Get the root domain (the registrable part)
    pub fn get_domain_root(url: &Url) -> Option<&str> {
        Self::get_domain(url).and_then(|x| x.root())
    }
}
