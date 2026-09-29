//! The currencies Lumen converts between (F9).
//!
//! # Where the table comes from
//!
//! Generated once, by hand, from two sources, and edited here from then on:
//!
//! - **The codes** are the 166 that ExchangeRate-API's open endpoint answers
//!   with. A rate for a code missing here is dropped: a row needs a name.
//! - **The countries and the decimals** are ISO 4217's list one, published
//!   2026-09-17, with the names cleaned ("NETHERLANDS (THE)" is
//!   "Netherlands") and a few everyday ones added -- "UK", "USA", "Russia",
//!   "Vietnam" -- because nobody types "Russian Federation" to find a rouble.
//!   Codes ISO has retired but the endpoint still quotes (ANG, BGN, HRK, ...)
//!   were given their countries by hand.
//! - **The display names** are ExchangeRate-API's, as the *Currency
//!   Exchange* extension for Raycast (MIT, by xeric) lists them: "Albanian
//!   Lek" rather than ISO's bare "Lek".
//!
//! A static table rather than a file: it changes when a country changes its
//! money, which is rarer than Lumen is released, and a table in the binary is
//! one that cannot fail to load.

/// One currency.
#[derive(Debug, PartialEq, Eq)]
pub struct Currency {
    /// ISO 4217, upper case.
    pub code: &'static str,
    pub name: &'static str,
    /// Digits after the decimal point in everyday amounts: 2 for the dollar,
    /// 0 for the yen, 3 for the Bahraini dinar.
    pub decimals: u8,
    /// Every country or territory that uses it, and common names for them.
    /// Searched, never shown.
    pub countries: &'static [&'static str],
}

impl Currency {
    /// The two letters drawn in the row's icon square: the code's first two,
    /// which for most currencies are the country's ISO 3166 code. Windows
    /// ships no flag emoji, so letters are what a flag would have been.
    pub fn region(&self) -> &'static str {
        &self.code[..2]
    }

    /// The symbol written before an amount, where one is common enough that
    /// people read it faster than the code. The rest are written with their
    /// code, as `AED 25.92`.
    pub fn symbol(&self) -> Option<&'static str> {
        Some(match self.code {
            "USD" => "$",
            "CAD" => "CA$",
            "AUD" => "A$",
            "NZD" => "NZ$",
            "HKD" => "HK$",
            "MXN" => "MX$",
            "BRL" => "R$",
            "EUR" => "\u{20ac}",
            "GBP" => "\u{a3}",
            "JPY" => "\u{a5}",
            "CNY" => "CN\u{a5}",
            "INR" => "\u{20b9}",
            "KRW" => "\u{20a9}",
            "ILS" => "\u{20aa}",
            "VND" => "\u{20ab}",
            "PHP" => "\u{20b1}",
            "TWD" => "NT$",
            _ => return None,
        })
    }
}

const fn currency(
    code: &'static str,
    name: &'static str,
    decimals: u8,
    countries: &'static [&'static str],
) -> Currency {
    Currency {
        code,
        name,
        decimals,
        countries,
    }
}

/// Every currency, by code.
pub static CURRENCIES: &[Currency] = &[
    currency(
        "AED",
        "UAE Dirham",
        2,
        &["United Arab Emirates", "UAE", "Emirates", "Dubai"],
    ),
    currency("AFN", "Afghan Afghani", 2, &["Afghanistan"]),
    currency("ALL", "Albanian Lek", 2, &["Albania"]),
    currency("AMD", "Armenian Dram", 2, &["Armenia"]),
    currency(
        "ANG",
        "Netherlands Antillean Guilder",
        2,
        &["Curaçao", "Sint Maarten"],
    ),
    currency("AOA", "Angolan Kwanza", 2, &["Angola"]),
    currency("ARS", "Argentine Peso", 2, &["Argentina"]),
    currency(
        "AUD",
        "Australian Dollar",
        2,
        &[
            "Australia",
            "Christmas Island",
            "Cocos Islands",
            "Heard Island and Mcdonald Islands",
            "Kiribati",
            "Nauru",
            "Norfolk Island",
            "Tuvalu",
        ],
    ),
    currency("AWG", "Aruban Florin", 2, &["Aruba"]),
    currency("AZN", "Azerbaijani Manat", 2, &["Azerbaijan"]),
    currency(
        "BAM",
        "Bosnia and Herzegovina Mark",
        2,
        &["Bosnia and Herzegovina"],
    ),
    currency("BBD", "Barbados Dollar", 2, &["Barbados"]),
    currency("BDT", "Bangladeshi Taka", 2, &["Bangladesh"]),
    currency("BGN", "Bulgarian Lev", 2, &["Bulgaria"]),
    currency("BHD", "Bahraini Dinar", 3, &["Bahrain"]),
    currency("BIF", "Burundian Franc", 0, &["Burundi"]),
    currency("BMD", "Bermudian Dollar", 2, &["Bermuda"]),
    currency("BND", "Brunei Dollar", 2, &["Brunei Darussalam"]),
    currency("BOB", "Bolivian Boliviano", 2, &["Bolivia"]),
    currency("BRL", "Brazilian Real", 2, &["Brazil"]),
    currency("BSD", "Bahamian Dollar", 2, &["Bahamas"]),
    currency("BTN", "Bhutanese Ngultrum", 2, &["Bhutan"]),
    currency("BWP", "Botswana Pula", 2, &["Botswana"]),
    currency("BYN", "Belarusian Ruble", 2, &["Belarus"]),
    currency("BZD", "Belize Dollar", 2, &["Belize"]),
    currency("CAD", "Canadian Dollar", 2, &["Canada"]),
    currency(
        "CDF",
        "Congolese Franc",
        2,
        &["Congo", "DR Congo", "Democratic Republic of the Congo"],
    ),
    currency("CHF", "Swiss Franc", 2, &["Liechtenstein", "Switzerland"]),
    currency("CLF", "Chilean Unit of Account (UF)", 4, &["Chile"]),
    currency("CLP", "Chilean Peso", 0, &["Chile"]),
    currency("CNH", "Chinese Renminbi (Offshore)", 2, &["China"]),
    currency("CNY", "Chinese Renminbi", 2, &["China", "Mainland China"]),
    currency("COP", "Colombian Peso", 2, &["Colombia"]),
    currency("CRC", "Costa Rican Colon", 2, &["Costa Rica"]),
    currency("CUP", "Cuban Peso", 2, &["Cuba"]),
    currency("CVE", "Cape Verdean Escudo", 2, &["Cabo Verde"]),
    currency("CZK", "Czech Koruna", 2, &["Czechia", "Czech Republic"]),
    currency("DJF", "Djiboutian Franc", 0, &["Djibouti"]),
    currency(
        "DKK",
        "Danish Krone",
        2,
        &["Denmark", "Faroe Islands", "Greenland"],
    ),
    currency("DOP", "Dominican Peso", 2, &["Dominican Republic"]),
    currency("DZD", "Algerian Dinar", 2, &["Algeria"]),
    currency("EGP", "Egyptian Pound", 2, &["Egypt"]),
    currency("ERN", "Eritrean Nakfa", 2, &["Eritrea"]),
    currency("ETB", "Ethiopian Birr", 2, &["Ethiopia"]),
    currency(
        "EUR",
        "Euro",
        2,
        &[
            "Åland Islands",
            "Andorra",
            "Austria",
            "Belgium",
            "Bulgaria",
            "Croatia",
            "Cyprus",
            "Estonia",
            "European Union",
            "Finland",
            "France",
            "French Guiana",
            "French Southern Territories",
            "Germany",
            "Greece",
            "Guadeloupe",
            "Holy See",
            "Ireland",
            "Italy",
            "Latvia",
            "Lithuania",
            "Luxembourg",
            "Malta",
            "Martinique",
            "Mayotte",
            "Monaco",
            "Montenegro",
            "Netherlands",
            "Portugal",
            "Réunion",
            "Saint Barthélemy",
            "Saint Martin",
            "Saint Pierre and Miquelon",
            "San Marino",
            "Slovakia",
            "Slovenia",
            "Spain",
            "Europe",
            "Eurozone",
            "Vatican",
        ],
    ),
    currency("FJD", "Fiji Dollar", 2, &["Fiji"]),
    currency(
        "FKP",
        "Falkland Islands Pound",
        2,
        &["Falkland Islands [malvinas]", "Falklands"],
    ),
    currency("FOK", "Faroese Króna", 2, &["Faroe Islands"]),
    currency(
        "GBP",
        "Pound Sterling",
        2,
        &[
            "Guernsey",
            "Isle of Man",
            "Jersey",
            "United Kingdom of Great Britain and Northern Ireland",
            "UK",
            "Britain",
            "Great Britain",
            "England",
            "Scotland",
            "Wales",
            "Northern Ireland",
        ],
    ),
    currency("GEL", "Georgian Lari", 2, &["Georgia"]),
    currency("GGP", "Guernsey Pound", 2, &["Guernsey"]),
    currency("GHS", "Ghanaian Cedi", 2, &["Ghana"]),
    currency("GIP", "Gibraltar Pound", 2, &["Gibraltar"]),
    currency("GMD", "Gambian Dalasi", 2, &["Gambia"]),
    currency("GNF", "Guinean Franc", 0, &["Guinea"]),
    currency("GTQ", "Guatemalan Quetzal", 2, &["Guatemala"]),
    currency("GYD", "Guyanese Dollar", 2, &["Guyana"]),
    currency("HKD", "Hong Kong Dollar", 2, &["Hong Kong"]),
    currency("HNL", "Honduran Lempira", 2, &["Honduras"]),
    currency("HRK", "Croatian Kuna", 2, &["Croatia"]),
    currency("HTG", "Haitian Gourde", 2, &["Haiti"]),
    currency("HUF", "Hungarian Forint", 2, &["Hungary"]),
    currency("IDR", "Indonesian Rupiah", 2, &["Indonesia"]),
    currency("ILS", "Israeli New Shekel", 2, &["Israel"]),
    currency("IMP", "Manx Pound", 2, &["Isle of Man"]),
    currency("INR", "Indian Rupee", 2, &["Bhutan", "India"]),
    currency("IQD", "Iraqi Dinar", 3, &["Iraq"]),
    currency("IRR", "Iranian Rial", 2, &["Iran"]),
    currency("ISK", "Icelandic Króna", 0, &["Iceland"]),
    currency("JEP", "Jersey Pound", 2, &["Jersey"]),
    currency("JMD", "Jamaican Dollar", 2, &["Jamaica"]),
    currency("JOD", "Jordanian Dinar", 3, &["Jordan"]),
    currency("JPY", "Japanese Yen", 0, &["Japan"]),
    currency("KES", "Kenyan Shilling", 2, &["Kenya"]),
    currency("KGS", "Kyrgyzstani Som", 2, &["Kyrgyzstan"]),
    currency("KHR", "Cambodian Riel", 2, &["Cambodia"]),
    currency("KID", "Kiribati Dollar", 2, &["Kiribati"]),
    currency("KMF", "Comorian Franc", 0, &["Comoros"]),
    currency("KRW", "South Korean Won", 0, &["Korea", "South Korea"]),
    currency("KWD", "Kuwaiti Dinar", 3, &["Kuwait"]),
    currency("KYD", "Cayman Islands Dollar", 2, &["Cayman Islands"]),
    currency("KZT", "Kazakhstani Tenge", 2, &["Kazakhstan"]),
    currency(
        "LAK",
        "Lao Kip",
        2,
        &["Lao People’s Democratic Republic", "Laos"],
    ),
    currency("LBP", "Lebanese Pound", 2, &["Lebanon"]),
    currency("LKR", "Sri Lanka Rupee", 2, &["Sri Lanka"]),
    currency("LRD", "Liberian Dollar", 2, &["Liberia"]),
    currency("LSL", "Lesotho Loti", 2, &["Lesotho"]),
    currency("LYD", "Libyan Dinar", 3, &["Libya"]),
    currency("MAD", "Moroccan Dirham", 2, &["Morocco", "Western Sahara"]),
    currency("MDL", "Moldovan Leu", 2, &["Moldova"]),
    currency("MGA", "Malagasy Ariary", 2, &["Madagascar"]),
    currency("MKD", "Macedonian Denar", 2, &["North Macedonia"]),
    currency("MMK", "Burmese Kyat", 2, &["Myanmar"]),
    currency("MNT", "Mongolian Tögrög", 2, &["Mongolia"]),
    currency("MOP", "Macanese Pataca", 2, &["Macao", "Macau"]),
    currency("MRU", "Mauritanian Ouguiya", 2, &["Mauritania"]),
    currency("MUR", "Mauritian Rupee", 2, &["Mauritius"]),
    currency("MVR", "Maldivian Rufiyaa", 2, &["Maldives"]),
    currency("MWK", "Malawian Kwacha", 2, &["Malawi"]),
    currency("MXN", "Mexican Peso", 2, &["Mexico"]),
    currency("MYR", "Malaysian Ringgit", 2, &["Malaysia"]),
    currency("MZN", "Mozambican Metical", 2, &["Mozambique"]),
    currency("NAD", "Namibian Dollar", 2, &["Namibia"]),
    currency("NGN", "Nigerian Naira", 2, &["Nigeria"]),
    currency("NIO", "Nicaraguan Córdoba", 2, &["Nicaragua"]),
    currency(
        "NOK",
        "Norwegian Krone",
        2,
        &["Bouvet Island", "Norway", "Svalbard and Jan Mayen"],
    ),
    currency("NPR", "Nepalese Rupee", 2, &["Nepal"]),
    currency(
        "NZD",
        "New Zealand Dollar",
        2,
        &["Cook Islands", "New Zealand", "Niue", "Pitcairn", "Tokelau"],
    ),
    currency("OMR", "Omani Rial", 3, &["Oman"]),
    currency("PAB", "Panamanian Balboa", 2, &["Panama"]),
    currency("PEN", "Peruvian Sol", 2, &["Peru"]),
    currency("PGK", "Papua New Guinean Kina", 2, &["Papua New Guinea"]),
    currency("PHP", "Philippine Peso", 2, &["Philippines"]),
    currency("PKR", "Pakistani Rupee", 2, &["Pakistan"]),
    currency("PLN", "Polish Złoty", 2, &["Poland"]),
    currency("PYG", "Paraguayan Guaraní", 0, &["Paraguay"]),
    currency("QAR", "Qatari Riyal", 2, &["Qatar"]),
    currency("RON", "Romanian Leu", 2, &["Romania"]),
    currency("RSD", "Serbian Dinar", 2, &["Serbia"]),
    currency("RUB", "Russian Ruble", 2, &["Russian Federation", "Russia"]),
    currency("RWF", "Rwandan Franc", 0, &["Rwanda"]),
    currency("SAR", "Saudi Riyal", 2, &["Saudi Arabia"]),
    currency("SBD", "Solomon Islands Dollar", 2, &["Solomon Islands"]),
    currency("SCR", "Seychellois Rupee", 2, &["Seychelles"]),
    currency("SDG", "Sudanese Pound", 2, &["Sudan"]),
    currency("SEK", "Swedish Krona", 2, &["Sweden"]),
    currency("SGD", "Singapore Dollar", 2, &["Singapore"]),
    currency("SHP", "Saint Helena Pound", 2, &["Saint Helena"]),
    currency("SLE", "Sierra Leonean Leone", 2, &["Sierra Leone"]),
    currency("SLL", "Sierra Leonean Leone (Old)", 2, &["Sierra Leone"]),
    currency("SOS", "Somali Shilling", 2, &["Somalia"]),
    currency("SRD", "Surinamese Dollar", 2, &["Suriname"]),
    currency("SSP", "South Sudanese Pound", 2, &["South Sudan"]),
    currency(
        "STN",
        "São Tomé and Príncipe Dobra",
        2,
        &["Sao Tome and Principe"],
    ),
    currency("SYP", "Syrian Pound", 2, &["Syrian Arab Republic", "Syria"]),
    currency("SZL", "Eswatini Lilangeni", 2, &["Eswatini"]),
    currency("THB", "Thai Baht", 2, &["Thailand"]),
    currency("TJS", "Tajikistani Somoni", 2, &["Tajikistan"]),
    currency("TMT", "Turkmenistan Manat", 2, &["Turkmenistan"]),
    currency("TND", "Tunisian Dinar", 3, &["Tunisia"]),
    currency("TOP", "Tongan Paʻanga", 2, &["Tonga"]),
    currency("TRY", "Turkish Lira", 2, &["Türki̇ye", "Turkey"]),
    currency(
        "TTD",
        "Trinidad and Tobago Dollar",
        2,
        &["Trinidad and Tobago"],
    ),
    currency("TVD", "Tuvaluan Dollar", 2, &["Tuvalu"]),
    currency("TWD", "New Taiwan Dollar", 2, &["Taiwan"]),
    currency("TZS", "Tanzanian Shilling", 2, &["Tanzania"]),
    currency("UAH", "Ukrainian Hryvnia", 2, &["Ukraine"]),
    currency("UGX", "Ugandan Shilling", 0, &["Uganda"]),
    currency(
        "USD",
        "United States Dollar",
        2,
        &[
            "American Samoa",
            "Bonaire",
            "British Indian Ocean Territory",
            "Ecuador",
            "El Salvador",
            "Guam",
            "Haiti",
            "Marshall Islands",
            "Micronesia",
            "Northern Mariana Islands",
            "Palau",
            "Panama",
            "Puerto Rico",
            "Timor-Leste",
            "Turks and Caicos Islands",
            "United States Minor Outlying Islands",
            "United States of America",
            "Virgin Islands",
            "USA",
            "America",
            "United States",
        ],
    ),
    currency("UYU", "Uruguayan Peso", 2, &["Uruguay"]),
    currency("UZS", "Uzbekistani So'm", 2, &["Uzbekistan"]),
    currency("VES", "Venezuelan Bolívar Soberano", 2, &["Venezuela"]),
    currency("VND", "Vietnamese Đồng", 0, &["Viet Nam", "Vietnam"]),
    currency("VUV", "Vanuatu Vatu", 0, &["Vanuatu"]),
    currency("WST", "Samoan Tālā", 2, &["Samoa"]),
    currency(
        "XAF",
        "Central African CFA Franc",
        0,
        &[
            "Cameroon",
            "Central African Republic",
            "Chad",
            "Congo",
            "Equatorial Guinea",
            "Gabon",
            "Republic of the Congo",
        ],
    ),
    currency(
        "XCD",
        "East Caribbean Dollar",
        2,
        &[
            "Anguilla",
            "Antigua and Barbuda",
            "Dominica",
            "Grenada",
            "Montserrat",
            "Saint Kitts and Nevis",
            "Saint Lucia",
            "Saint Vincent and the Grenadines",
        ],
    ),
    currency("XCG", "Caribbean Guilder", 2, &["Curaçao", "Sint Maarten"]),
    currency("XDR", "Special Drawing Rights", 2, &[]),
    currency(
        "XOF",
        "West African CFA franc",
        0,
        &[
            "Benin",
            "Burkina Faso",
            "Côte d'Ivoire",
            "Guinea-Bissau",
            "Mali",
            "Niger",
            "Senegal",
            "Togo",
            "Ivory Coast",
        ],
    ),
    currency(
        "XPF",
        "CFP Franc",
        0,
        &["French Polynesia", "New Caledonia", "Wallis and Futuna"],
    ),
    currency("YER", "Yemeni Rial", 2, &["Yemen"]),
    currency(
        "ZAR",
        "South African Rand",
        2,
        &["Lesotho", "Namibia", "South Africa"],
    ),
    currency("ZMW", "Zambian Kwacha", 2, &["Zambia"]),
    currency("ZWG", "Zimbabwe Gold", 2, &["Zimbabwe"]),
    currency("ZWL", "Zimbabwean Dollar", 2, &["Zimbabwe"]),
];

/// The currency with this code, in any case.
pub fn find(code: &str) -> Option<&'static Currency> {
    let code = code.trim();
    if code.len() != 3 {
        return None;
    }
    let code = code.to_ascii_uppercase();
    CURRENCIES
        .binary_search_by(|currency| currency.code.cmp(code.as_str()))
        .ok()
        .map(|index| &CURRENCIES[index])
}

/// The one currency `text` names exactly, in any case: its code, its name, or
/// a country that uses it. `None` when it names none, or several -- Zimbabwe
/// has two -- because a root answer must not guess (F9, slice 5).
pub fn exactly(text: &str) -> Option<&'static Currency> {
    let text = text.trim();
    if let Some(currency) = find(text) {
        return Some(currency);
    }
    let mut named = CURRENCIES.iter().filter(|currency| {
        currency.name.eq_ignore_ascii_case(text)
            || currency
                .countries
                .iter()
                .any(|country| country.eq_ignore_ascii_case(text))
    });
    let first = named.next()?;
    named.next().is_none().then_some(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_currency_is_named_exactly_by_code_name_or_country() {
        assert_eq!(exactly("cad").unwrap().code, "CAD");
        assert_eq!(exactly("canadian dollar").unwrap().code, "CAD");
        assert_eq!(exactly("Japan").unwrap().code, "JPY");
        assert_eq!(exactly("spain").unwrap().code, "EUR");
        assert!(exactly("can").is_none(), "a prefix is not a name");
        assert!(exactly("zimbabwe").is_none(), "two currencies");
    }

    /// `find` searches by halves, which is only right if the table is sorted.
    #[test]
    fn the_table_is_sorted_by_code_with_no_repeats() {
        for pair in CURRENCIES.windows(2) {
            assert!(
                pair[0].code < pair[1].code,
                "{} {}",
                pair[0].code,
                pair[1].code
            );
        }
    }

    #[test]
    fn codes_are_three_upper_case_letters() {
        for currency in CURRENCIES {
            assert!(
                currency.code.len() == 3 && currency.code.bytes().all(|b| b.is_ascii_uppercase()),
                "{}",
                currency.code
            );
        }
    }

    #[test]
    fn a_code_is_found_in_any_case() {
        assert_eq!(find("cad").unwrap().name, "Canadian Dollar");
        assert_eq!(find(" EUR ").unwrap().code, "EUR");
        assert!(find("xyz").is_none());
        assert!(find("dollar").is_none());
    }

    #[test]
    fn minor_units_come_from_iso() {
        assert_eq!(find("USD").unwrap().decimals, 2);
        assert_eq!(find("JPY").unwrap().decimals, 0);
        assert_eq!(find("BHD").unwrap().decimals, 3);
    }

    #[test]
    fn the_euro_knows_its_countries() {
        let euro = find("EUR").unwrap();
        for country in ["Spain", "Germany", "Ireland", "Croatia"] {
            assert!(euro.countries.contains(&country), "{country}");
        }
        assert_eq!(euro.region(), "EU");
    }
}
