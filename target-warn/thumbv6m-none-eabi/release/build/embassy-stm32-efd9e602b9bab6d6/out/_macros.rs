macro_rules ! set_clocks { ($ ($ (# [$ m : meta]) * $ k : ident : $ v : expr ,) *) => { { # [allow (unused)] struct Temp { $ ($ (# [$ m]) * $ k : Option < crate :: time :: Hertz > ,) * } let all = Temp { $ ($ (# [$ m]) * $ k : $ v ,) * } ; crate :: rcc :: set_freqs (crate :: rcc :: Clocks { hclk1 : all . hclk1 . into () , hsi : all . hsi . into () , lse : all . lse . into () , lsi : all . lsi . into () , pclk1 : all . pclk1 . into () , pclk1_tim : all . pclk1_tim . into () , pclk2 : all . pclk2 . into () , pclk2_tim : all . pclk2_tim . into () , rtc : all . rtc . into () , sys : all . sys . into () , }) ; } } ; }
#[allow(unused)]
macro_rules! foreach_flash_region {
    ($($pat:tt => $code:tt;)*) => {
        macro_rules! __foreach_flash_region_inner {
            $(($pat) => $code;)*
            ($_:tt) => {}
        }
        __foreach_flash_region_inner!((Bank1Region,4,128));
    };
}
#[allow(unused)]
macro_rules! foreach_interrupt {
    ($($pat:tt => $code:tt;)*) => {
        macro_rules! __foreach_interrupt_inner {
            $(($pat) => $code;)*
            ($_:tt) => {}
        }
        __foreach_interrupt_inner!((ADC1,adc,ADC,GLOBAL,ADC1_COMP));
        __foreach_interrupt_inner!((DMA1,bdma,DMA,CH1,DMA1_CHANNEL1));
        __foreach_interrupt_inner!((DMA1,bdma,DMA,CH2,DMA1_CHANNEL2_3));
        __foreach_interrupt_inner!((DMA1,bdma,DMA,CH3,DMA1_CHANNEL2_3));
        __foreach_interrupt_inner!((DMA1,bdma,DMA,CH4,DMA1_CHANNEL4_5_6_7));
        __foreach_interrupt_inner!((DMA1,bdma,DMA,CH5,DMA1_CHANNEL4_5_6_7));
        __foreach_interrupt_inner!((DMA1,bdma,DMA,CH6,DMA1_CHANNEL4_5_6_7));
        __foreach_interrupt_inner!((DMA1,bdma,DMA,CH7,DMA1_CHANNEL4_5_6_7));
        __foreach_interrupt_inner!((EXTI,exti,EXTI,EXTI0,EXTI0_1));
        __foreach_interrupt_inner!((EXTI,exti,EXTI,EXTI1,EXTI0_1));
        __foreach_interrupt_inner!((EXTI,exti,EXTI,EXTI10,EXTI4_15));
        __foreach_interrupt_inner!((EXTI,exti,EXTI,EXTI11,EXTI4_15));
        __foreach_interrupt_inner!((EXTI,exti,EXTI,EXTI12,EXTI4_15));
        __foreach_interrupt_inner!((EXTI,exti,EXTI,EXTI13,EXTI4_15));
        __foreach_interrupt_inner!((EXTI,exti,EXTI,EXTI14,EXTI4_15));
        __foreach_interrupt_inner!((EXTI,exti,EXTI,EXTI15,EXTI4_15));
        __foreach_interrupt_inner!((EXTI,exti,EXTI,EXTI2,EXTI2_3));
        __foreach_interrupt_inner!((EXTI,exti,EXTI,EXTI3,EXTI2_3));
        __foreach_interrupt_inner!((EXTI,exti,EXTI,EXTI4,EXTI4_15));
        __foreach_interrupt_inner!((EXTI,exti,EXTI,EXTI5,EXTI4_15));
        __foreach_interrupt_inner!((EXTI,exti,EXTI,EXTI6,EXTI4_15));
        __foreach_interrupt_inner!((EXTI,exti,EXTI,EXTI7,EXTI4_15));
        __foreach_interrupt_inner!((EXTI,exti,EXTI,EXTI8,EXTI4_15));
        __foreach_interrupt_inner!((EXTI,exti,EXTI,EXTI9,EXTI4_15));
        __foreach_interrupt_inner!((FLASH,flash,FLASH,GLOBAL,FLASH));
        __foreach_interrupt_inner!((I2C1,i2c,I2C,ER,I2C1));
        __foreach_interrupt_inner!((I2C1,i2c,I2C,EV,I2C1));
        __foreach_interrupt_inner!((I2C2,i2c,I2C,ER,I2C2));
        __foreach_interrupt_inner!((I2C2,i2c,I2C,EV,I2C2));
        __foreach_interrupt_inner!((LPTIM1,lptim,LPTIM,GLOBAL,LPTIM1));
        __foreach_interrupt_inner!((LPUART1,usart,LPUART,GLOBAL,LPUART1));
        __foreach_interrupt_inner!((RCC,rcc,RCC,GLOBAL,RCC));
        __foreach_interrupt_inner!((RTC,rtc,RTC,ALARM,RTC));
        __foreach_interrupt_inner!((RTC,rtc,RTC,SSRU,RTC));
        __foreach_interrupt_inner!((RTC,rtc,RTC,STAMP,RTC));
        __foreach_interrupt_inner!((RTC,rtc,RTC,TAMP,RTC));
        __foreach_interrupt_inner!((RTC,rtc,RTC,WKUP,RTC));
        __foreach_interrupt_inner!((SPI1,spi,SPI,GLOBAL,SPI1));
        __foreach_interrupt_inner!((SPI2,spi,SPI,GLOBAL,SPI2));
        __foreach_interrupt_inner!((TIM2,timer,TIM_GP16,BRK,TIM2));
        __foreach_interrupt_inner!((TIM2,timer,TIM_GP16,CC,TIM2));
        __foreach_interrupt_inner!((TIM2,timer,TIM_GP16,COM,TIM2));
        __foreach_interrupt_inner!((TIM2,timer,TIM_GP16,TRG,TIM2));
        __foreach_interrupt_inner!((TIM2,timer,TIM_GP16,UP,TIM2));
        __foreach_interrupt_inner!((TIM21,timer,TIM_2CH,BRK,TIM21));
        __foreach_interrupt_inner!((TIM21,timer,TIM_2CH,CC,TIM21));
        __foreach_interrupt_inner!((TIM21,timer,TIM_2CH,COM,TIM21));
        __foreach_interrupt_inner!((TIM21,timer,TIM_2CH,TRG,TIM21));
        __foreach_interrupt_inner!((TIM21,timer,TIM_2CH,UP,TIM21));
        __foreach_interrupt_inner!((TIM22,timer,TIM_2CH,BRK,TIM22));
        __foreach_interrupt_inner!((TIM22,timer,TIM_2CH,CC,TIM22));
        __foreach_interrupt_inner!((TIM22,timer,TIM_2CH,COM,TIM22));
        __foreach_interrupt_inner!((TIM22,timer,TIM_2CH,TRG,TIM22));
        __foreach_interrupt_inner!((TIM22,timer,TIM_2CH,UP,TIM22));
        __foreach_interrupt_inner!((TIM6,timer,TIM_BASIC,BRK,TIM6));
        __foreach_interrupt_inner!((TIM6,timer,TIM_BASIC,CC,TIM6));
        __foreach_interrupt_inner!((TIM6,timer,TIM_BASIC,COM,TIM6));
        __foreach_interrupt_inner!((TIM6,timer,TIM_BASIC,TRG,TIM6));
        __foreach_interrupt_inner!((TIM6,timer,TIM_BASIC,UP,TIM6));
        __foreach_interrupt_inner!((USART1,usart,USART,GLOBAL,USART1));
        __foreach_interrupt_inner!((USART2,usart,USART,GLOBAL,USART2));
        __foreach_interrupt_inner!((WWDG,wwdg,WWDG,GLOBAL,WWDG));
        __foreach_interrupt_inner!((WWDG,wwdg,WWDG,RST,WWDG));
        __foreach_interrupt_inner!((WWDG));
        __foreach_interrupt_inner!((PVD));
        __foreach_interrupt_inner!((RTC));
        __foreach_interrupt_inner!((FLASH));
        __foreach_interrupt_inner!((RCC));
        __foreach_interrupt_inner!((EXTI0_1));
        __foreach_interrupt_inner!((EXTI,EXTI0_1));
        __foreach_interrupt_inner!((EXTI2_3));
        __foreach_interrupt_inner!((EXTI,EXTI2_3));
        __foreach_interrupt_inner!((EXTI4_15));
        __foreach_interrupt_inner!((EXTI,EXTI4_15));
        __foreach_interrupt_inner!((DMA1_CHANNEL1));
        __foreach_interrupt_inner!((DMA1_CHANNEL2_3));
        __foreach_interrupt_inner!((DMA1_CHANNEL4_5_6_7));
        __foreach_interrupt_inner!((ADC1_COMP));
        __foreach_interrupt_inner!((LPTIM1));
        __foreach_interrupt_inner!((TIM2));
        __foreach_interrupt_inner!((TIM6));
        __foreach_interrupt_inner!((TIM21));
        __foreach_interrupt_inner!((TIM22));
        __foreach_interrupt_inner!((I2C1));
        __foreach_interrupt_inner!((I2C2));
        __foreach_interrupt_inner!((SPI1));
        __foreach_interrupt_inner!((SPI2));
        __foreach_interrupt_inner!((USART1));
        __foreach_interrupt_inner!((USART2));
        __foreach_interrupt_inner!((LPUART1));
    };
}
#[allow(unused)]
macro_rules! foreach_peripheral {
    ($($pat:tt => $code:tt;)*) => {
        macro_rules! __foreach_peripheral_inner {
            $(($pat) => $code;)*
            ($_:tt) => {}
        }
        __foreach_peripheral_inner!((adc,ADC1));
        __foreach_peripheral_inner!((crc,CRC));
        __foreach_peripheral_inner!((dbgmcu,DBGMCU));
        __foreach_peripheral_inner!((bdma,DMA1));
        __foreach_peripheral_inner!((exti,EXTI));
        __foreach_peripheral_inner!((flash,FLASH));
        __foreach_peripheral_inner!((gpio,GPIOA));
        __foreach_peripheral_inner!((gpio,GPIOB));
        __foreach_peripheral_inner!((gpio,GPIOC));
        __foreach_peripheral_inner!((gpio,GPIOD));
        __foreach_peripheral_inner!((gpio,GPIOH));
        __foreach_peripheral_inner!((i2c,I2C1));
        __foreach_peripheral_inner!((i2c,I2C2));
        __foreach_peripheral_inner!((iwdg,IWDG));
        __foreach_peripheral_inner!((lptim,LPTIM1));
        __foreach_peripheral_inner!((usart,LPUART1));
        __foreach_peripheral_inner!((pwr,PWR));
        __foreach_peripheral_inner!((rcc,RCC));
        __foreach_peripheral_inner!((rtc,RTC));
        __foreach_peripheral_inner!((spi,SPI1));
        __foreach_peripheral_inner!((spi,SPI2));
        __foreach_peripheral_inner!((syscfg,SYSCFG));
        __foreach_peripheral_inner!((timer,TIM2));
        __foreach_peripheral_inner!((timer,TIM21));
        __foreach_peripheral_inner!((timer,TIM22));
        __foreach_peripheral_inner!((timer,TIM6));
        __foreach_peripheral_inner!((uid,UID));
        __foreach_peripheral_inner!((usart,USART1));
        __foreach_peripheral_inner!((usart,USART2));
        __foreach_peripheral_inner!((wwdg,WWDG));
    };
}
#[allow(unused)]
macro_rules! foreach_pin {
    ($($pat:tt => $code:tt;)*) => {
        macro_rules! __foreach_pin_inner {
            $(($pat) => $code;)*
            ($_:tt) => {}
        }
        __foreach_pin_inner!((PA0,GPIOA,0,0,EXTI0));
        __foreach_pin_inner!((PA1,GPIOA,0,1,EXTI1));
        __foreach_pin_inner!((PA2,GPIOA,0,2,EXTI2));
        __foreach_pin_inner!((PA3,GPIOA,0,3,EXTI3));
        __foreach_pin_inner!((PA4,GPIOA,0,4,EXTI4));
        __foreach_pin_inner!((PA5,GPIOA,0,5,EXTI5));
        __foreach_pin_inner!((PA6,GPIOA,0,6,EXTI6));
        __foreach_pin_inner!((PA7,GPIOA,0,7,EXTI7));
        __foreach_pin_inner!((PA8,GPIOA,0,8,EXTI8));
        __foreach_pin_inner!((PA9,GPIOA,0,9,EXTI9));
        __foreach_pin_inner!((PA10,GPIOA,0,10,EXTI10));
        __foreach_pin_inner!((PA11,GPIOA,0,11,EXTI11));
        __foreach_pin_inner!((PA12,GPIOA,0,12,EXTI12));
        __foreach_pin_inner!((PA13,GPIOA,0,13,EXTI13));
        __foreach_pin_inner!((PA14,GPIOA,0,14,EXTI14));
        __foreach_pin_inner!((PA15,GPIOA,0,15,EXTI15));
        __foreach_pin_inner!((PB0,GPIOB,1,0,EXTI0));
        __foreach_pin_inner!((PB1,GPIOB,1,1,EXTI1));
        __foreach_pin_inner!((PB2,GPIOB,1,2,EXTI2));
        __foreach_pin_inner!((PB3,GPIOB,1,3,EXTI3));
        __foreach_pin_inner!((PB4,GPIOB,1,4,EXTI4));
        __foreach_pin_inner!((PB5,GPIOB,1,5,EXTI5));
        __foreach_pin_inner!((PB6,GPIOB,1,6,EXTI6));
        __foreach_pin_inner!((PB7,GPIOB,1,7,EXTI7));
        __foreach_pin_inner!((PB8,GPIOB,1,8,EXTI8));
        __foreach_pin_inner!((PB9,GPIOB,1,9,EXTI9));
        __foreach_pin_inner!((PB10,GPIOB,1,10,EXTI10));
        __foreach_pin_inner!((PB11,GPIOB,1,11,EXTI11));
        __foreach_pin_inner!((PB12,GPIOB,1,12,EXTI12));
        __foreach_pin_inner!((PB13,GPIOB,1,13,EXTI13));
        __foreach_pin_inner!((PB14,GPIOB,1,14,EXTI14));
        __foreach_pin_inner!((PB15,GPIOB,1,15,EXTI15));
        __foreach_pin_inner!((PC13,GPIOC,2,13,EXTI13));
        __foreach_pin_inner!((PC14,GPIOC,2,14,EXTI14));
        __foreach_pin_inner!((PC15,GPIOC,2,15,EXTI15));
        __foreach_pin_inner!((PH0,GPIOH,7,0,EXTI0));
        __foreach_pin_inner!((PH1,GPIOH,7,1,EXTI1));
    };
}
#[allow(unused)]
macro_rules! foreach_adc {
    ($($pat:tt => $code:tt;)*) => {
        macro_rules! __foreach_adc_inner {
            $(($pat) => $code;)*
            ($_:tt) => {}
        }
        __foreach_adc_inner!((ADC1,ADC1_COMMON,adc));
    };
}
