// © 2026 Massachusetts Institute of Technology
// MIT License

#include <limits.h>
#include <stdio.h>
#include <stdlib.h>

void printLine (const char * line)
{
    if(line != NULL) 
    {
        printf("%s\n", line);
    }
}

void printHexCharLine (char charHex)
{
    printf("%02x\n", charHex);
}

void bad()
{
    char data;
    data = CHAR_MAX;
    if(data > 0)
    {
        char result = data * 2;
        printHexCharLine(result);
    }
}

static void goodG2B()
{
    char data;
    data = 2;
    if(data > 0)
    {
        char result = data * 2;
        printHexCharLine(result);
    }
}

static void goodB2G()
{
    char data;
    data = ' ';
    data = CHAR_MAX;
    if(data > 0)
    {
        if (data < (CHAR_MAX/2))
        {
            char result = data * 2;
            printHexCharLine(result);
        }
        else
        {
            printLine("data value is too large to perform arithmetic safely.");
        }
    }
}

void good()
{
    goodG2B();
    goodB2G();
}

int main() {
    int x = 0;
    scanf("%d", &x);

    if (x)
    {
        good();
    }
    else
    {
        bad();
    }
    return 0;
}
