#include <stdio.h>
#include <string.h>
#include <curl/curl.h>

int main(void) {
    CURLcode rc = curl_global_init(CURL_GLOBAL_DEFAULT);
    curl_global_cleanup();
    printf("init %d\n", (int)rc);
    printf("prefix %d\n", strncmp(curl_version(), "libcurl", 7) == 0 ? 1 : 0);
    return 0;
}
