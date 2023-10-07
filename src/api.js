import { setAll } from 'silkjs';

const BASE_URL = "";

var timestampOffset = 10000000;
// initial value is large to force sync
var timezoneOffset = 0;

export function timestamp() {
  return new Date().getTime() + timestampOffset
}

export function timezoneSecs() {
  return timezoneOffset;
}

export function postEvent(event, data, options) {
  options = options || {};
  data = data || {};
  data.event = event;

  var requestOptions = {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
    },
    body: JSON.stringify(data),
  };

  for (var prop in options) {
    if (options.hasOwnProperty(prop)) {
      requestOptions[prop] = options[prop];
    }
  }

  var xhr = new XMLHttpRequest();
  xhr.open(requestOptions.method, BASE_URL + "/events");

  // xhr.setRequestHeader("Content-Type", "application/json");

  for (var header in requestOptions.headers) {
    if (requestOptions.headers.hasOwnProperty(header)) {
      xhr.setRequestHeader(header, requestOptions.headers[header]);
    }
  }

  xhr.send(requestOptions.body);

  return xhr;
}


function poll(cnt) {
  var xhr = new XMLHttpRequest();
  xhr.open('GET', '/updates?cnt=' + cnt + '&timestamp=' + timestamp(), true);
  xhr.timeout = 10000; // Set timeout to 10 seconds

  xhr.onload = function () {
    if (this.status == 204) {
      // NOTE: This is basically a keep-alive 
      // server times out after 5s, handle it gracefully
      // console.error('Long-polling request timed out on the server');
      setTimeout(() => poll(cnt), 0); // retry immediately with the same state
    } else if (this.status >= 200 && this.status < 300) {
      var response = JSON.parse(xhr.responseText);

      if (response.cnt == -1) {
        timestampOffset += response.offset || 0;
        timezoneOffset = response.tzOffset || 0;
        // log("info", "update", {offset: response.offset, tzOffset: response.tzOffset});
        setTimeout(() => poll(cnt), 0); // reschedule immediately 
      } else {
        setAll(response.update, false); // fail hard flag
        setTimeout(() => poll(response.cnt), 0); // reschedule immediately 
      }
    } else {
      console.error('Long-polling request failed');
      setTimeout(() => poll(0), 5000); // retry after 5 seconds
    }
  };

  xhr.ontimeout = function () {
    console.error('Long-polling request timed out');
    setTimeout(() => poll(0), 5000); // retry after 5 seconds
  };

  xhr.onerror = function () {
    console.error('Long-polling request failed');
    setTimeout(() => poll(0), 5000); // retry after 5 seconds
  };

  console.log("Poll: timestamp=" + timestamp());
  xhr.send();
}

document.addEventListener('DOMContentLoaded', () => {
  // trace("info", "start polling");
  console.log("start polling");
  poll(0);
})

window.onerror = function (message, source, lineno, colno, error) {
  var info = "An error occurred: " + message;
  info += "\nSource: " + source;
  info += "\nLine Number: " + lineno;
  info += "\nColumn Number: " + colno;

  // Check if the browser supports error.stack and if so, add it to the info
  if (error && error.stack) {
    info += "\nStack trace: " + error.stack;
  }

  log(info);

  return true; // If you return true, the error won't be reported in the console
}

export function trace(level, message, data) {
  const now = new Date(timestamp());
  var xhr = new XMLHttpRequest();
  xhr.open('POST', '/log', true);
  xhr.setRequestHeader('Content-Type', 'application/json');
  xhr.send(JSON.stringify({
    level: level,
    message: message,
    data: data || {},
    // timestamp: new Date().getTime()
    timestamp: now.toISOString()
  }));
}
