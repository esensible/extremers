#include <SPI.h>
#include <WiFiNINA.h>
#include <vector>
#include "utility/server_drv.h"

extern "C" {
  #include "utility/debug.h"
}

extern "C" {
  extern void init_engine();
  extern int32_t handle_request_ffi(const uint8_t* request, size_t request_len, uint8_t* response, size_t response_len, void (*sleep_fn)(size_t, size_t));
}

void sleep(size_t time, size_t pos) { }

char ssid[] = "yournetwork";
char pass[] = "yourpassword";

int status = WL_IDLE_STATUS;
int _sock;
int _lastSock = NO_SOCKET_AVAIL;

uint16_t _port = 80;
const int requestBufferSize = 2048;

struct PartialClient {
  WiFiClient* client;
  char currentRequest[requestBufferSize];
  int currentOffset = 0;
};

std::vector<PartialClient> partial_clients;
std::vector<WiFiClient*> pending_clients;
unsigned long startMillis;
const unsigned long delayMillis = 3000;

void led() {
  digitalWrite(LED_BUILTIN, HIGH);
  delay(1000);
  digitalWrite(LED_BUILTIN, LOW);
  delay(1000);
}

void led0() {
  digitalWrite(LED_BUILTIN, HIGH);
  delay(20);
  digitalWrite(LED_BUILTIN, LOW);
  delay(20);
}

bool isClientNew(WiFiClient& client) {
  for (auto& partial_client : partial_clients) {
    if (partial_client.client->remoteIP() == client.remoteIP() &&
        partial_client.client->remotePort() == client.remotePort()) {
      return false;
    }
  }
  return true;
}


void setup() {
  pinMode(LED_BUILTIN, OUTPUT);
  Serial.begin(115200);
  while (!Serial) led();

  if (WiFi.status() == WL_NO_MODULE) while (true);

  if (WiFi.firmwareVersion() < WIFI_FIRMWARE_LATEST_VERSION) {
    Serial.println("Please upgrade the firmware");
  }

  status = WiFi.beginAP(ssid, pass);
  if (status != WL_AP_LISTENING) while (true);

  delay(10000);

  _sock = ServerDrv::getSocket();
  if (_sock != NO_SOCKET_AVAIL) {
    ServerDrv::startServer(_port, _sock);
  }

  init_engine();

  Serial.println("setup complete");
}

void loop() {
  if (status != WiFi.status()) {
    status = WiFi.status();
  }

  if (!pending_clients.empty()) {
    if (millis() - startMillis >= delayMillis) {
      for (const char *line : {"HTTP/1.1 200 OK", "Content-type:text/html", "", "<html><body><h1>World, hello!</h1></body></html>", ""}) {
        for (WiFiClient *client : pending_clients) {
          client->println(line);
        }
      }
      for (WiFiClient *client : pending_clients) {
        client->stop();
        delete client;
      }
      pending_clients.clear();
    }
  }

  int sock = ServerDrv::availServer(_sock);

  if (sock != NO_SOCKET_AVAIL) {
    WiFiClient client(sock);
    if (isClientNew(client)) {
      partial_clients.push_back({new WiFiClient(client), {'\0'}, 0});
    }
  }

  for (auto it = partial_clients.begin(); it != partial_clients.end();) {
    uint8_t clientStatus = it->client->status();

    if (clientStatus == CLOSED || clientStatus == CLOSE_WAIT || clientStatus == FIN_WAIT_1 || clientStatus == FIN_WAIT_2) {
      delete it->client;
      it = partial_clients.erase(it);
      continue;
    }

    int bytesToRead = it->client->available();

    if (bytesToRead > 0) {

      it->client->read((uint8_t *)&it->currentRequest[it->currentOffset], bytesToRead);
      it->currentOffset += bytesToRead;

      // Check for an empty line at the end of the HTTP header
      // if (it->currentOffset >= 4 && ((strncmp(&it->currentRequest[it->currentOffset - 4], "\r\n\r\n", 4) == 0) || 
      //    (it->currentOffset >= 2 && strncmp(&it->currentRequest[it->currentOffset - 2], "\n\n", 2) == 0))) {

          char response_buf[1024] = {0};
          int result = handle_request_ffi((const uint8_t*)it->currentRequest, it->currentOffset, (uint8_t*)response_buf, bytesToRead, sleep);
          if (result == 0) {
            Serial.println(response_buf);
          }


        pending_clients.push_back(it->client);
        it = partial_clients.erase(it);
        if (pending_clients.size() == 1) {
          startMillis = millis();
        }
        continue;
      // }
    }
    ++it;
  }
}