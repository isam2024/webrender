<?php

declare(strict_types=1);

namespace Shotwright\Exception;

class RenderException extends ShotwrightException
{
    public function __construct(
        string $message,
        public readonly int $exitCode,
        public readonly string $stderr,
    ) {
        parent::__construct($message);
    }
}
