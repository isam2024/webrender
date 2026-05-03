<?php

declare(strict_types=1);

namespace Shotwright\Tests;

use PHPUnit\Framework\TestCase;
use Shotwright\Exception\ShotwrightException;
use Shotwright\Options;

final class OptionsTest extends TestCase
{
    public function testDefaultsAreReasonable(): void
    {
        $o = new Options();
        $this->assertSame(1280, $o->width);
        $this->assertSame(800, $o->height);
        $this->assertFalse($o->fullPage);
    }

    public function testFromArrayMapsAllFields(): void
    {
        $o = Options::fromArray([
            'width' => 1024,
            'height' => 768,
            'full_page' => true,
            'timeout' => 45,
            'dpr' => 2.0,
            'settle_ms' => 500,
            'user_agent' => 'Mozilla/5.0 (custom)',
        ]);
        $this->assertSame(1024, $o->width);
        $this->assertSame(768, $o->height);
        $this->assertTrue($o->fullPage);
        $this->assertSame(45, $o->timeoutSeconds);
        $this->assertSame(2.0, $o->devicePixelRatio);
        $this->assertSame(500, $o->settleMilliseconds);
        $this->assertSame('Mozilla/5.0 (custom)', $o->userAgent);
    }

    public function testRejectsNonPositiveDimensions(): void
    {
        $this->expectException(ShotwrightException::class);
        new Options(width: 0);
    }

    public function testRejectsExcessiveViewport(): void
    {
        $this->expectException(ShotwrightException::class);
        new Options(width: 9999);
    }

    public function testRejectsAbsurdTimeout(): void
    {
        $this->expectException(ShotwrightException::class);
        new Options(timeoutSeconds: 1000);
    }

    public function testToCliArgsIncludesFullPageFlagOnlyWhenSet(): void
    {
        $args = (new Options(fullPage: false))->toCliArgs();
        $this->assertNotContains('--full-page', $args);

        $args = (new Options(fullPage: true))->toCliArgs();
        $this->assertContains('--full-page', $args);
    }

    public function testToCliArgsHasNumericValuesAsStrings(): void
    {
        $args = (new Options(width: 1280, height: 800))->toCliArgs();
        $i = array_search('--width', $args, true);
        $this->assertIsInt($i);
        $this->assertSame('1280', $args[$i + 1]);
    }
}
