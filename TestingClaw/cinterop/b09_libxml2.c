#include <stdio.h>
#include <libxml/parser.h>

int main(void) {
    printf("len %d\n", xmlStrlen((const xmlChar *)"hello"));
    int c = xmlStrcmp((const xmlChar *)"abc", (const xmlChar *)"abd");
    printf("cmp %d\n", c < 0 ? -1 : (c > 0 ? 1 : 0));
    xmlDocPtr doc = xmlReadMemory("<a><b/></a>", 11, NULL, NULL, 0);
    int ok = 0;
    if (doc != NULL && xmlDocGetRootElement(doc) != NULL) ok = 1;
    xmlFreeDoc(doc);
    printf("root %d\n", ok);
    return 0;
}
