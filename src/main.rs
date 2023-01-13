use warp::{hyper::body::Bytes, Filter, Rejection, http::Response, http::StatusCode};
use std::{collections::HashMap, sync::{Arc, Mutex, MutexGuard}, time::{Instant, Duration}};
use reqwest;

#[tokio::main]
async fn main() {

    // initially tried using a single hashmap but couldn't figure out how to move it into two separate closures
    // which I needed in order to access it both before and after the conditional reqwest call in .and_then
    let last_visited: Arc<Mutex<HashMap<String, Instant>>> = Arc::new(Mutex::new(HashMap::new()));
    let cached_responses: Arc<Mutex<HashMap<String, Response<Bytes>>>> = Arc::new(Mutex::new(HashMap::new()));
    
    // conditional call to api passed in query parameter
    let handler = move |params: HashMap<String, String>| async move {
        if params.contains_key("query") && params.contains_key("visited") {
            let url: String = params.get("query").unwrap().to_string();
            let response: Response<Bytes> = Response::builder()
                .status(StatusCode::from_u16(203).unwrap())
                .header("url", url.clone())
                .body(Bytes::from(url))
                .unwrap();
            Ok::<_, Rejection>(response)
        } else if params.contains_key("query") {
            let call: &String = params.get("query").unwrap();
            let result = reqwest::get(call).await;
            match result {
                Ok(response) => {
                    let status = response.status();
                    let body = response.bytes().await.unwrap();
                    let response = Response::builder()
                        .status(status)
                        .header("url", call.to_string().clone())
                        .body(body)
                        .unwrap();
                    Ok::<_, Rejection>(response)
                }
                Err(err) => {
                    let response = Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Bytes::from(err.to_string()))
                        .unwrap();
                    Ok::<_, Rejection>(response)
                }
            }
        } else {
            let response = Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Bytes::from("query parameter not found"))
                .unwrap();
            Ok::<_, Rejection>(response)
        }
    };
    let last_visited_clone = last_visited.clone();
    // captures/updates the last visited time for a url
    let last_visited_closure = move |params: HashMap<String, String>| {
        let url: String = params.get("query").unwrap().to_string();
        let mut last_visited: MutexGuard<HashMap<String, Instant>> = last_visited.lock().unwrap();
        let now: Instant = Instant::now();
        let elapsed: Duration = now - *last_visited.entry(url.clone()).or_insert(now);
        let _last: Instant = *last_visited.entry(url.clone()).and_modify(|last: &mut Instant| *last = now ).or_insert(now);
        elapsed
    };

    // store responses in a cache
    let cached_responses_closure = move |key: String, value: Response<Bytes>| {
        let mut last_clone = last_visited_clone.lock().unwrap();
        let mut cached_responses: MutexGuard<HashMap<String, Response<Bytes>>> = cached_responses.lock().unwrap();
        let a: Bytes = cached_responses.entry(key.clone()).or_insert(value).body().clone();
        a
    };
    
    // get parameters from the query string and add visited if the last visit occurred less than 30 seconds ago
    let query_params = warp::query::<HashMap<String, String>>().map(move |params: HashMap<String, String>| {
        let mut mut_params: HashMap<String, String> = params.clone();
        let time_since_last_visit: Duration = last_visited_closure(params.clone());
        if Duration::from_secs(0) < time_since_last_visit && time_since_last_visit < Duration::from_secs(30) {
            mut_params.insert("visited".to_string(), params.get("query").unwrap().to_string());
        }
        mut_params
    });

    // this may violate the condition of not using dependencies in the caching layer as it relies on warp's Response::builder method to clone the response
    let map_response = move |a: Response<Bytes>| {
        let status_code: StatusCode = a.status();
        let headers: String = a.headers().get("url").unwrap().to_str().unwrap().to_string();
        if status_code == StatusCode::from_u16(203).unwrap() {
            let from_cache: Bytes = cached_responses_closure(headers, a);
            return Response::builder().status(status_code).header("from-cache", "yes").body(from_cache).unwrap();
        } else {
            let to_cache: Response<Bytes> = Response::builder().status(status_code).body(a.body().clone()).unwrap();
            cached_responses_closure(headers, to_cache);
            return a;
        }
    };
    
     //handles all paths the same way. pass api endpoint to query parameter (e.g. localhost:3030/?query=https://blockstream.info/api/blocks)
    // if from cache, the response will have a header "from-cache: yes"
    let route = warp::any()
        .and(query_params)
        .and_then(handler)
        .map(map_response);
         
    // start the server
    warp::serve(route).run(([0, 0, 0, 0], 3030)).await;
    
}

