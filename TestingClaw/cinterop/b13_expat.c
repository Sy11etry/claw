#include <stdio.h>
#include <expat.h>

static int count = 0;

static void on_start(void *ud, const XML_Char *name, const XML_Char **atts) {
    (void)ud; (void)name; (void)atts;
    count++;
}

int main(void) {
    XML_Parser p = XML_ParserCreate(NULL);
    XML_SetStartElementHandler(p, on_start);
    XML_Parse(p, "<a><b/><c><d/></c></a>", 22, 1);
    XML_ParserFree(p);
    printf("elements %d\n", count);
    return 0;
}
