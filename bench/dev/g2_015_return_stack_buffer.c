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

static char *helperBad()
{
    char charString[] = "helperBad string";
    return charString;
}

void bad() 
{
    printLine(helperBad());
}

static char *helperGood1()
{
    static char charString[] = "helperGood1 string";
    return charString;
}

void good() 
{
    printLine(helperGood1());
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
