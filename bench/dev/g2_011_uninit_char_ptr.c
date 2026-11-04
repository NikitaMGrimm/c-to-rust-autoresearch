// © 2026 Massachusetts Institute of Technology
// MIT License

#include <stdio.h>

void printLine(const char *line)
{
    if (line != NULL)
    {
        printf("%s\n", line);
    }
}

void bad()
{
    char *data;
    printLine(data);
}

void good()
{
    char *data;
    data = "string";
    printLine(data);
}

int main()
{
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
