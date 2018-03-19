use std::collections::HashMap;
use std::io::Read;

use yaml_rust::Yaml;
use colored::*;
use serde_json;
use time;

use hyper::client::{Client, Response};
use hyper::net::HttpsConnector;
use hyper_native_tls::NativeTlsClient;
use hyper::header::{UserAgent, Headers, Cookie, SetCookie};
use hyper::method::Method;

use interpolator;

use actions::{Runnable, Report};

static USER_AGENT: &'static str = "drill";

#[derive(Clone)]
pub struct Request {
  name: String,
  url: String,
  time: f64,
  method: String,
  headers: HashMap<String, String>,
  pub body: Option<String>,
  pub with_item: Option<Yaml>,
  pub assign: Option<String>,
}

impl Request {
  pub fn is_that_you(item: &Yaml) -> bool{
    item["request"].as_hash().is_some()
  }

  pub fn new(item: &Yaml, with_item: Option<Yaml>) -> Request {
    let reference: Option<&str> = item["assign"].as_str();
    let body: Option<&str> = item["request"]["body"].as_str();
    let method;

    let mut headers = HashMap::new();

    if let Some(v) = item["request"]["method"].as_str() {
      method = v.to_string().to_uppercase();
    } else {
      method = "GET".to_string();
    }

    if let Some(hash) = item["request"]["headers"].as_hash() {
      for (key, val) in hash.iter() {
        headers.insert(key.as_str().unwrap().to_string(), val.as_str().unwrap().to_string());
      }
    }

    Request {
      name: item["name"].as_str().unwrap().to_string(),
      url: item["request"]["url"].as_str().unwrap().to_string(),
      time: 0.0,
      method: method,
      headers: headers,
      body: body.map(str::to_string),
      with_item: with_item,
      assign: reference.map(str::to_string),
    }
  }

  fn send_request(&self, context: &mut HashMap<String, Yaml>, responses: &mut HashMap<String, serde_json::Value>) -> (Response, f64) {
    let mut ssl = NativeTlsClient::new().unwrap();

    ssl.danger_disable_hostname_verification(true);

    let connector = HttpsConnector::new(ssl);
    let client = Client::with_connector(connector);

    let begin = time::precise_time_s();

    let interpolated_url;
    let interpolated_body;
    let request;

    // Resolve the url
    {
      let interpolator = interpolator::Interpolator::new(context, responses);
      interpolated_url = interpolator.resolve(&self.url);
    }

    // Method
    let method = match self.method.to_uppercase().as_ref() {
      "GET" => Method::Get,
      "POST" => Method::Post,
      "PUT" => Method::Put,
      "PATCH" => Method::Patch,
      "DELETE" => Method::Delete,
      _ => panic!("Unknown method '{}'", self.method),
    };

    // Body
    if let Some(body) = self.body.as_ref() {
      // Resolve the body
      let interpolator = interpolator::Interpolator::new(context, responses);
      interpolated_body = interpolator.resolve(body).to_owned();

      request = client
        .request(method, &interpolated_url)
        .body(&interpolated_body);
    } else {
      request = client.request(method, &interpolated_url);
    }

    // Headers
    let mut headers = Headers::new();
    headers.set(UserAgent(USER_AGENT.to_string()));

    for (key, val) in self.headers.iter() {
      // Resolve the body
      let interpolator = interpolator::Interpolator::new(context, responses);
      let interpolated_header = interpolator.resolve(val).to_owned();

      headers.set_raw(key.to_owned(), vec![interpolated_header.clone().into_bytes()]);
    }

    if let Some(cookie) = context.get("cookie") {
      headers.set(Cookie(vec![String::from(cookie.as_str().unwrap())]));
    }

    let response_result = request.headers(headers).send();

    if let Err(e) = response_result {
      panic!("Error connecting '{}': {:?}", interpolated_url, e);
    }

    let response = response_result.unwrap();
    let duration_ms = (time::precise_time_s() - begin) * 1000.0;

    println!("{:width$} {} {} {}{}", self.name.green(), interpolated_url.blue().bold(), response.status.to_string().yellow(), duration_ms.round().to_string().cyan(), "ms".cyan(), width=25);

    (response, duration_ms)
  }
}

impl Runnable for Request {
  fn execute(&self, context: &mut HashMap<String, Yaml>, responses: &mut HashMap<String, serde_json::Value>, reports: &mut Vec<Report>) {
    if self.with_item.is_some() {
      context.insert("item".to_string(), self.with_item.clone().unwrap());
    }

    let (mut response, duration_ms) = self.send_request(context, responses);

    reports.push(Report { name: self.name.to_owned(), duration: duration_ms, status: response.status.to_u16() });

    if let Some(&SetCookie(ref cookies)) = response.headers.get::<SetCookie>() {
      if let Some(cookie) = cookies.iter().next() {
        let value = String::from(cookie.split(";").next().unwrap());
        context.insert("cookie".to_string(), Yaml::String(value));
      }
    }

    if let Some(ref key) = self.assign {
      let mut data = String::new();

      response.read_to_string(&mut data).unwrap();

      let value: serde_json::Value = serde_json::from_str(&data).unwrap();

      responses.insert(key.to_owned(), value);
    }
  }

}
