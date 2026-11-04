// © 2026 Massachusetts Institute of Technology
// MIT License

#include <stdio.h>

void printHexCharLine (char charHex)
{
    printf("%02x\n", charHex);
}

int main()
{
    char data;
    data = ' ';
    fscanf (stdin, "%c", &data);
    {
        char result = data + 1;
        printHexCharLine(result);
    }
    
    return 0;
}