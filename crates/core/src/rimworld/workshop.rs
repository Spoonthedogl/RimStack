use crate::base::error::Error;
use crate::said;

use crate::base::vdf::{Token, tokens};
use rayon::prelude::*;
use serde_json::Value;
use std::collections::HashMap;

const DETAILS_URL: &str =
    "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/";

/// `timeupdated` in `appworkshop_294100.acf`: seconds since 1970, by Workshop id.
pub fn installed_times(acf: &str) -> HashMap<String, u64> {
    installed(acf, "timeupdated")
}

pub fn installed_sizes(acf: &str) -> HashMap<String, u64> {
    installed(acf, "size")
}

fn installed(acf: &str, fact: &str) -> HashMap<String, u64> {
    let all = tokens(acf);
    let mut times = HashMap::new();
    let start = all
        .iter()
        .position(|t| matches!(t, Token::Text(s) if s == "WorkshopItemsInstalled"));
    let Some(start) = start else {
        return times;
    };
    // "<id>" { "key" "value" ... } repeated, inside one pair of braces.
    let mut depth = 0;
    let mut item: Option<&str> = None;
    let mut rest = all[start + 1..].iter();
    while let Some(token) = rest.next() {
        match token {
            Token::Open => depth += 1,
            Token::Close => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            Token::Text(text) if depth == 1 => item = Some(text),
            Token::Text(key) => {
                let value = rest.next();
                if let (2, true, Some(id), Some(Token::Text(v))) = (depth, key == fact, item, value)
                    && let Ok(time) = v.parse()
                {
                    times.insert(id.to_string(), time);
                }
            }
        }
    }
    times
}

pub fn ids_in(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for piece in text.split(|c: char| c.is_whitespace() || c == ',' || c == ';') {
        let digits = |s: &str| -> String { s.chars().take_while(char::is_ascii_digit).collect() };
        let id = match piece.split_once("id=") {
            Some((_, after)) => digits(after),
            None if piece.chars().all(|c| c.is_ascii_digit()) => piece.to_string(),
            None => String::new(),
        };
        if (6..=12).contains(&id.len()) && !found.contains(&id) {
            found.push(id);
        }
    }
    found
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Details {
    pub id: String,
    pub title: String,
    /// Seconds since 1970.
    pub updated: u64,
    /// 0 when Steam does not say.
    pub size: u64,
}

pub fn parse_details(json: &str) -> Result<Vec<Details>, Error> {
    let root: Value = serde_json::from_str(json)
        .map_err(|e| Error::format(said!("Steam's answer is not JSON: {}", e)))?;
    let files = root
        .pointer("/response/publishedfiledetails")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::format(said!("Steam's answer has no file list")))?;
    let details = |file: &Value| -> Option<Details> {
        Some(Details {
            id: file.get("publishedfileid")?.as_str()?.to_string(),
            title: file.get("title")?.as_str()?.to_string(),
            updated: file.get("time_updated")?.as_u64()?,
            // Steam writes the size as text.
            size: file
                .get("file_size")
                .and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok()))
                .unwrap_or(0),
        })
    };
    Ok(files.iter().filter_map(details).collect())
}

/// Blocks on the network. Steam is slow per item, so chunks of 100 go out in parallel.
pub fn fetch_details(ids: &[String]) -> Result<Vec<Details>, Error> {
    let agent = crate::base::web::agent(120);
    let ask = |batch: &[String]| -> Result<Vec<Details>, Error> {
        let mut form = vec![("itemcount".to_string(), batch.len().to_string())];
        for (i, id) in batch.iter().enumerate() {
            form.push((format!("publishedfileids[{i}]"), id.clone()));
        }
        let pairs = form.iter().map(|(k, v)| (k.as_str(), v.as_str()));
        let mut response = agent
            .post(DETAILS_URL)
            .send_form(pairs)
            .map_err(crate::base::web::steam_failed)?;
        // Descriptions come along, so this is above the default limit.
        let mut body = response
            .body_mut()
            .with_config()
            .limit(64 * 1024 * 1024)
            .reader();
        let mut text = String::new();
        std::io::Read::read_to_string(&mut body, &mut text)
            .map_err(crate::base::web::steam_failed)?;
        parse_details(&text)
    };
    let answers: Vec<Result<Vec<Details>, Error>> = ids.par_chunks(100).map(ask).collect();
    let mut all = Vec::new();
    for answer in answers {
        all.extend(answer?);
    }
    Ok(all)
}

const COLLECTIONS_URL: &str =
    "https://api.steampowered.com/ISteamRemoteStorage/GetCollectionDetails/v1/";

pub fn parse_collections(json: &str) -> Result<Vec<(String, Vec<String>)>, Error> {
    let root: Value = serde_json::from_str(json)
        .map_err(|e| Error::format(said!("Steam's answer is not JSON: {}", e)))?;
    let all = root["response"]["collectiondetails"].as_array();
    let text = |v: &Value| v["publishedfileid"].as_str().map(String::from);
    let one = |c: &Value| {
        let children = c["children"].as_array();
        Some((
            text(c)?,
            children.map_or(Vec::new(), |list| list.iter().filter_map(text).collect()),
        ))
    };
    Ok(all
        .ok_or_else(|| Error::format(said!("Steam's answer has no collections")))?
        .iter()
        .filter_map(one)
        .collect())
}

/// Blocks on the network.
pub fn expand_collections(ids: &[String]) -> Result<Vec<String>, Error> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let agent = crate::base::web::agent(60);
    let mut form = vec![("collectioncount".to_string(), ids.len().to_string())];
    for (i, id) in ids.iter().enumerate() {
        form.push((format!("publishedfileids[{i}]"), id.clone()));
    }
    let failed = crate::base::web::steam_failed;
    let pairs = form.iter().map(|(k, v)| (k.as_str(), v.as_str()));
    let mut response = agent
        .post(COLLECTIONS_URL)
        .send_form(pairs)
        .map_err(failed)?;
    let text = response.body_mut().read_to_string().map_err(failed)?;
    let collections = parse_collections(&text)?;
    let mut all: Vec<String> = Vec::new();
    for id in ids {
        let items = collections
            .iter()
            .find(|(c, items)| c == id && !items.is_empty());
        let these = items.map_or(std::slice::from_ref(id), |(_, items)| items.as_slice());
        for item in these {
            if !all.contains(item) {
                all.push(item.clone());
            }
        }
    }
    Ok(all)
}

const SEARCH_URL: &str = "https://steamcommunity.com/workshop/browse/";

/// The mods a Workshop search page lists, best match first. Links quoted in
/// descriptions are not `href`s, so they are not taken.
pub fn search_results(html: &str) -> Vec<String> {
    const LINK: &str = "href=\"https://steamcommunity.com/sharedfiles/filedetails/?id=";
    let mut found: Vec<String> = Vec::new();
    for after in html.split(LINK).skip(1) {
        let id: String = after.chars().take_while(char::is_ascii_digit).collect();
        if !id.is_empty() && !found.contains(&id) {
            found.push(id);
        }
    }
    found
}

/// Blocks on the network. The Workshop's own search page, which needs no key.
pub fn search(name: &str) -> Result<Vec<String>, Error> {
    let failed = crate::base::web::steam_failed;
    let mut response = crate::base::web::agent(30)
        .get(SEARCH_URL)
        .query("appid", "294100")
        .query("searchtext", name)
        .query("browsesort", "textsearch")
        .query("section", "readytouseitems")
        .call()
        .map_err(failed)?;
    let html = response.body_mut().read_to_string().map_err(failed)?;
    Ok(search_results(&html))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACF: &str = r#""AppWorkshop"
{
	"appid"		"294100"
	"TimeLastUpdated"		"1791750395"
	"WorkshopItemsInstalled"
	{
		"705924057"
		{
			"size"		"1372735"
			"timeupdated"		"1711286985"
			"manifest"		"7900175110102980905"
		}
		"712141500"
		{
			"size"		"5310730"
			"timeupdated"		"1710519878"
		}
	}
	"WorkshopItemDetails"
	{
		"705924057"
		{
			"timeupdated"		"5"
		}
	}
}"#;

    #[test]
    fn installed_times_come_from_the_installed_block_only() {
        let times = installed_times(ACF);
        assert_eq!(times.len(), 2);
        assert_eq!(times["705924057"], 1711286985);
        assert_eq!(times["712141500"], 1710519878);
        assert!(installed_times("garbage {").is_empty());
        assert_eq!(installed_sizes(ACF)["705924057"], 1372735);
    }

    #[test]
    fn ids_are_taken_from_links_and_bare_numbers_once_each() {
        let pasted = "https://steamcommunity.com/sharedfiles/filedetails/?id=2009463077&searchtext=x\n\
                      818773962, 2009463077 hello 12 steam://url/CommunityFilePage/1 ?id=abc";
        assert_eq!(ids_in(pasted), ["2009463077", "818773962"]);
    }

    #[test]
    fn details_skip_items_steam_does_not_know() {
        let json = r#"{"response":{"result":1,"resultcount":2,"publishedfiledetails":[
            {"publishedfileid":"2009463077","result":1,"title":"Harmony","time_updated":1756914561,"file_size":"5729526"},
            {"publishedfileid":"1","result":9}]}}"#;
        let found = parse_details(json).unwrap();
        assert_eq!(
            found,
            [Details {
                id: "2009463077".into(),
                title: "Harmony".into(),
                updated: 1756914561,
                size: 5729526
            }]
        );
        assert!(parse_details("<html>").is_err());
        assert!(parse_details("{}").is_err());
    }

    #[test]
    #[ignore = "needs the network and RIMSTACK_WORKSHOP: .../steamapps/workshop/content/294100"]
    fn real_workshop_folder_is_checked_for_updates() {
        let content = std::path::PathBuf::from(std::env::var_os("RIMSTACK_WORKSHOP").unwrap());
        let record = content
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("appworkshop_294100.acf");
        let on_disk = installed_times(&std::fs::read_to_string(record).unwrap());
        let ids: Vec<String> = std::fs::read_dir(&content)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        let started = std::time::Instant::now();
        let now = fetch_details(&ids).unwrap();
        println!(
            "asked {} answered {} in {:?}",
            ids.len(),
            now.len(),
            started.elapsed()
        );
        let newer = |d: &&Details| on_disk.get(&d.id).is_some_and(|&have| d.updated > have);
        let behind: Vec<&Details> = now.iter().filter(newer).collect();
        println!("{} on record, {} behind", on_disk.len(), behind.len());
        for d in behind.iter().take(8) {
            println!("  {} have {} now {}", d.title, on_disk[&d.id], d.updated);
        }
        assert!(now.len() > ids.len() / 2);
    }

    #[test]
    #[ignore = "needs the network"]
    fn steam_answers_about_a_real_item() {
        let found = fetch_details(&["2009463077".to_string(), "1".to_string()]).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].title, "Harmony");
    }

    #[test]
    fn search_takes_each_result_once_and_not_links_in_descriptions() {
        let page = r#"{\"short_description\":\"https://steamcommunity.com/sharedfiles/filedetails/?id=999\"}
            <a href="https://steamcommunity.com/sharedfiles/filedetails/?id=1561769193" class="a"><img></a>
            <a href="https://steamcommunity.com/sharedfiles/filedetails/?id=1561769193" class="b">Common Sense</a>
            <a href="https://steamcommunity.com/sharedfiles/filedetails/?id=2822058610" class="b">Patch</a>
            <a href="https://steamcommunity.com/sharedfiles/filedetails/?id=" class="b">Broken</a>"#;
        assert_eq!(search_results(page), ["1561769193", "2822058610"]);
        assert!(search_results("<html></html>").is_empty());
    }

    #[test]
    #[ignore = "needs the network"]
    fn steam_finds_a_mod_by_name() {
        let found = search("Harmony").unwrap();
        assert!(found.contains(&"2009463077".to_string()));
    }

    #[test]
    fn a_collection_gives_its_items_and_a_mod_none() {
        let answer = r#"{"response":{"result":1,"resultcount":2,"collectiondetails":[
            {"publishedfileid":"111","result":1,"children":[{"publishedfileid":"5","sortorder":1,"filetype":0},{"publishedfileid":"6","sortorder":2,"filetype":0}]},
            {"publishedfileid":"222","result":9}]}}"#;
        let found = parse_collections(answer).unwrap();
        assert_eq!(
            found[0],
            ("111".to_string(), vec!["5".to_string(), "6".to_string()])
        );
        assert_eq!(found[1], ("222".to_string(), vec![]));
        assert!(parse_collections("{}").is_err());
    }
}
