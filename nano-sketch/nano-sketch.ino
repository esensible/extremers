#include <SPI.h>
#include <WiFiNINA.h>
#include <vector>
#include "utility/server_drv.h"

extern "C"
{
#include "utility/debug.h"
}

typedef void (*SleepFn)(size_t, size_t);

extern "C"
{
    int32_t handle_request_ffi(
        const uint8_t *request,
        size_t request_len,
        uint8_t *response,
        size_t *response_len,
        uint8_t *update,
        size_t *update_len,
        uint8_t **extra,
        size_t *extra_len,
        SleepFn sleep_fn);
}

void sleep(size_t time, size_t pos) {}

char ssid[] = "yournetwork";
char pass[] = "yourpassword";

int status = WL_IDLE_STATUS;
int _sock;
int _lastSock = NO_SOCKET_AVAIL;

uint16_t _port = 80;
const int requestBufferSize = 2048;

struct PartialClient
{
    WiFiClient *client;
    char currentRequest[requestBufferSize];
    int currentOffset = 0;
};

std::vector<PartialClient> partial_clients;
std::vector<WiFiClient *> pending_clients;
unsigned long startMillis;
const unsigned long delayMillis = 3000;

void led()
{
    digitalWrite(LED_BUILTIN, HIGH);
    delay(1000);
    digitalWrite(LED_BUILTIN, LOW);
    delay(1000);
}

void led0()
{
    digitalWrite(LED_BUILTIN, HIGH);
    delay(20);
    digitalWrite(LED_BUILTIN, LOW);
    delay(20);
}

bool isClientNew(WiFiClient &client)
{
    for (auto &partial_client : partial_clients)
    {
        if (partial_client.client->remoteIP() == client.remoteIP() &&
            partial_client.client->remotePort() == client.remotePort())
        {
            return false;
        }
    }
    return true;
}

void setup()
{
    pinMode(LED_BUILTIN, OUTPUT);
    Serial.begin(115200);
    while (!Serial)
        led();

    if (WiFi.status() == WL_NO_MODULE)
        while (true)
            ;

    if (WiFi.firmwareVersion() < WIFI_FIRMWARE_LATEST_VERSION)
    {
        Serial.println("Please upgrade the firmware");
    }

    status = WiFi.beginAP(ssid, pass);
    if (status != WL_AP_LISTENING)
        while (true)
            ;

    delay(10000);

    _sock = ServerDrv::getSocket();
    if (_sock != NO_SOCKET_AVAIL)
    {
        ServerDrv::startServer(_port, _sock);
    }

    Serial.println("setup complete");
}

void loop()
{
    if (status != WiFi.status())
    {
        status = WiFi.status();
    }

    int sock = ServerDrv::availServer(_sock);

    if (sock != NO_SOCKET_AVAIL)
    {
        WiFiClient client(sock);
        if (isClientNew(client))
        {
            partial_clients.push_back({new WiFiClient(client), {'\0'}, 0});
        }
    }

    for (auto it = partial_clients.begin(); it != partial_clients.end();)
    {
        uint8_t clientStatus = it->client->status();

        if (clientStatus == CLOSED || clientStatus == CLOSE_WAIT || clientStatus == FIN_WAIT_1 || clientStatus == FIN_WAIT_2)
        {
            delete it->client;
            it = partial_clients.erase(it);
            continue;
        }

        int bytesToRead = it->client->available();

        if (bytesToRead > 0)
        {

            it->client->read((uint8_t *)&it->currentRequest[it->currentOffset], bytesToRead);
            it->currentOffset += bytesToRead;

            // Check for an empty line at the end of the HTTP header
            // if (it->currentOffset >= 4 && ((strncmp(&it->currentRequest[it->currentOffset - 4], "\r\n\r\n", 4) == 0) ||
            //    (it->currentOffset >= 2 && strncmp(&it->currentRequest[it->currentOffset - 2], "\n\n", 2) == 0))) {

            char response_buf[1024] = {0};
            size_t response_len = sizeof(response_buf);
            char update_buf[1024] = {0};
            size_t update_len = sizeof(update_buf);
            char *extra_buf = NULL;
            size_t extra_len = 0;

            Serial.println(it->currentRequest);

            int result = handle_request_ffi(
                (const uint8_t *)it->currentRequest, it->currentOffset,
                (uint8_t *)response_buf, &response_len,
                (uint8_t *)update_buf, &update_len,
                (uint8_t **)&extra_buf, &extra_len,
                sleep);

            if (result == 0)
            {
                Serial.println(response_buf);

                if (response_len > 0)
                {
                    // probably a GET /static or POST /events with no change
                    it->client->write((uint8_t *)response_buf, response_len);
                    if (extra_len > 0)
                    {
#define BLOCK_SIZE 1024
#define BLOCK_DELAY 50
                        for (size_t offs = 0; offs < extra_len; offs += BLOCK_SIZE)
                        {
                            size_t to_read = BLOCK_SIZE;
                            if (offs + to_read > extra_len)
                            {
                                to_read = extra_len - offs;
                            }
                            it->client->write((uint8_t *)&extra_buf[offs], to_read);
                            delay(BLOCK_DELAY);
                        }
                    }
                    it->client->stop();
                    delete it->client;
                }

                if (update_len > 0)
                {
                    // POST /events with a change
                    // go notify the pending clients
                    for (WiFiClient *client : pending_clients)
                    {
                        client->write((uint8_t *)update_buf, update_len);
                    }
                    for (WiFiClient *client : pending_clients)
                    {
                        client->stop();
                        delete client;
                    }
                    pending_clients.clear();
                }

                // pending GET /updates - mark as pending
                if (response_len == 0 && update_len == 0 && extra_len == 0)
                {
                    Serial.write("pending...");
                    pending_clients.push_back(it->client);
                }
            }
            else
            {
                Serial.println(response_buf);
                it->client->write((uint8_t *)response_buf, response_len);
                it->client->stop();
                delete it->client;
            }

            it = partial_clients.erase(it);
            continue;
            // }
        }
        ++it;
    }
}