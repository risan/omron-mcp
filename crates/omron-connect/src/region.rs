//! Which OMRON cloud serves a country.
//!
//! OMRON runs two API systems. Countries on the older "v1" system talk to a
//! Kii-based cloud, one per area, each with its own app id and key. Countries
//! on the newer "v2" system talk to a regional REST API. A country is in at
//! most one of the two. Countries OMRON marks offline, and unknown codes, are
//! not supported.

use crate::error::{Error, Result};

/// A v1 (Kii) cloud: the API root and the app credentials every request carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct V1Region {
    pub cloud_url: &'static str,
    pub app_id: &'static str,
    pub app_key: &'static str,
}

/// A v2 region: the API roots to try in order (primary first).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct V2Region {
    pub name: &'static str,
    pub base_urls: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    V1(V1Region),
    V2(V2Region),
}

impl Region {
    /// Looks up an ISO 3166-1 alpha-2 country code, ignoring case and spaces.
    pub fn for_country(code: &str) -> Result<Region> {
        let code = code.trim().to_ascii_uppercase();

        if let Some(group) = V1_GROUPS
            .iter()
            .find(|group| group.countries.contains(&code.as_str()))
        {
            return Ok(Region::V1(group.region));
        }

        if let Some(group) = V2_GROUPS
            .iter()
            .find(|group| group.countries.contains(&code.as_str()))
        {
            return Ok(Region::V2(group.region));
        }

        Err(Error::UnsupportedCountry(code))
    }
}

struct V1Group {
    countries: &'static [&'static str],
    region: V1Region,
}

struct V2Group {
    countries: &'static [&'static str],
    region: V2Region,
}

const V1_GROUPS: &[V1Group] = &[
    V1Group {
        countries: &["AU", "BD", "ID", "MM", "MY", "NZ", "PH", "SG", "TH", "VN"],
        region: V1Region {
            cloud_url: "https://data-sg.omronconnect.com/api",
            app_id: "lou30y2xfa9f",
            app_key: "392a4bdff8af4141944d30ca8e3cc860",
        },
    },
    V1Group {
        countries: &["HK", "JP", "KR", "TW"],
        region: V1Region {
            cloud_url: "https://data-jp.omronconnect.com/api",
            app_id: "1e3ddd17",
            app_key: "b576cf704409ec86facdacc16fbaadad",
        },
    },
    V1Group {
        countries: &["IN"],
        region: V1Region {
            cloud_url: "https://data-in.omronconnect.com/api",
            app_id: "cuoy728n",
            app_key: "784uhaescyzyc28l2yh8xsth1xpga21g",
        },
    },
    V1Group {
        countries: &[
            "AR", "BO", "BR", "CL", "CO", "CR", "DO", "EC", "GT", "HN", "MX", "NI", "PA", "PE",
            "PY", "SV", "UY", "VE",
        ],
        region: V1Region {
            cloud_url: "https://data-eu.omronconnect.com/api",
            app_id: "bfyy2kf1d5a0",
            app_key: "989c6dbdc0244886ac2ba4de4892080e",
        },
    },
];

const V2_GROUPS: &[V2Group] = &[
    V2Group {
        countries: &[
            "AT", "BE", "BG", "CH", "CY", "CZ", "DE", "DK", "EE", "ES", "FI", "FR", "GB", "GR",
            "HR", "HU", "IE", "IT", "LT", "LU", "LV", "MT", "NL", "PL", "PT", "RO", "SE", "SI",
            "SK",
        ],
        region: V2Region {
            name: "EU",
            base_urls: &[
                "https://vlt-mobile-api.prd.eu.ohiomron.eu/prd",
                "https://oi-api.ohiomron.eu/app",
            ],
        },
    },
    V2Group {
        countries: &["AW", "BM", "BZ", "CA", "PR", "SR", "TT", "US"],
        region: V2Region {
            name: "NA",
            base_urls: &["https://vlt-mobile-api.prd.us.ohiomron.com/prd"],
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indonesia_is_on_the_singapore_v1_cloud() {
        let Region::V1(region) = Region::for_country("ID").unwrap() else {
            panic!("ID should be v1");
        };

        assert_eq!(region.cloud_url, "https://data-sg.omronconnect.com/api");
    }

    #[test]
    fn lookup_ignores_case_and_spaces() {
        assert_eq!(
            Region::for_country(" id ").unwrap(),
            Region::for_country("ID").unwrap()
        );
    }

    #[test]
    fn germany_is_v2_with_a_secondary_url() {
        let Region::V2(region) = Region::for_country("DE").unwrap() else {
            panic!("DE should be v2");
        };

        assert_eq!(region.name, "EU");
        assert_eq!(region.base_urls.len(), 2);
    }

    #[test]
    fn the_us_is_v2_with_one_url() {
        let Region::V2(region) = Region::for_country("US").unwrap() else {
            panic!("US should be v2");
        };

        assert_eq!(region.name, "NA");
        assert_eq!(region.base_urls.len(), 1);
    }

    #[test]
    fn offline_and_unknown_countries_are_unsupported() {
        for code in ["KH", "RU", "SA", "ZA", "ZZ", ""] {
            assert!(
                matches!(Region::for_country(code), Err(Error::UnsupportedCountry(_))),
                "{code} should be unsupported"
            );
        }
    }

    #[test]
    fn no_country_is_in_two_groups() {
        let mut seen = std::collections::HashSet::new();
        let v1 = V1_GROUPS.iter().flat_map(|group| group.countries);
        let v2 = V2_GROUPS.iter().flat_map(|group| group.countries);

        for code in v1.chain(v2) {
            assert!(seen.insert(*code), "{code} is listed twice");
        }

        assert_eq!(seen.len(), 70);
    }
}
