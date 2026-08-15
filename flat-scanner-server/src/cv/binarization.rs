use opencv::{
    core::{BORDER_DEFAULT, Mat, Point, Scalar, Size},
    imgproc,
    prelude::*,
};

pub fn apply_sauvola_threshold(src: &Mat, k: f32, window_size: i32) -> Result<Mat, String> {
    let mut gray = Mat::default();
    if src.channels() > 1 {
        imgproc::cvt_color(
            src,
            &mut gray,
            imgproc::COLOR_BGR2GRAY,
            0,
            opencv::core::AlgorithmHint::ALGO_HINT_APPROX,
        )
        .map_err(|e| e.to_string())?;
    } else {
        gray = src.clone();
    }

    let mut mean = Mat::default();
    let mut sq_mean = Mat::default();
    let mut variance = Mat::default();
    let mut std_dev = Mat::default();

    // 1. Расчет локального среднего m(x,y)
    imgproc::box_filter(
        &gray,
        &mut mean,
        -1,
        Size::new(window_size, window_size),
        Point::new(-1, -1),
        true,
        BORDER_DEFAULT,
    )
    .map_err(|e| e.to_string())?;

    let mut gray_f32 = Mat::default();
    gray.convert_to(&mut gray_f32, opencv::core::CV_32F, 1.0, 0.0)
        .map_err(|e| e.to_string())?;

    let mut mean_f32 = Mat::default();
    mean.convert_to(&mut mean_f32, opencv::core::CV_32F, 1.0, 0.0)
        .map_err(|e| e.to_string())?;

    // 2. Вычисление стандартного отклонения s(x,y)
    opencv::core::multiply(&gray_f32, &gray_f32, &mut sq_mean, 1.0, -1)
        .map_err(|e| e.to_string())?;
    imgproc::box_filter(
        &sq_mean,
        &mut variance,
        -1,
        Size::new(window_size, window_size),
        Point::new(-1, -1),
        true,
        BORDER_DEFAULT,
    )
    .map_err(|e| e.to_string())?;

    let mut mean_sq = Mat::default();
    opencv::core::multiply(&mean_f32, &mean_f32, &mut mean_sq, 1.0, -1)
        .map_err(|e| e.to_string())?;

    let mut diff = Mat::default();
    opencv::core::subtract(&variance, &mean_sq, &mut diff, &Mat::default(), -1)
        .map_err(|e| e.to_string())?;

    // Патч Коллизии: сохраняем результат максимума в новый буфер max_diff
    let mut max_diff = Mat::default();
    opencv::core::max(&diff, &Scalar::new(0.0, 0.0, 0.0, 0.0), &mut max_diff)
        .map_err(|e| e.to_string())?;
    opencv::core::sqrt(&max_diff, &mut std_dev).map_err(|e| e.to_string())?;

    // 3. Вычисление матрицы порогов Сауволы с раздельными буферами
    let mut t_factor1 = Mat::default();
    let scale_factor = 1.0 / 128.0;

    // Умножаем на 1/128
    opencv::core::multiply(
        &std_dev,
        &Scalar::new(scale_factor.into(), 0.0, 0.0, 0.0),
        &mut t_factor1,
        1.0,
        -1,
    )
    .map_err(|e| e.to_string())?;

    // Вычитаем 1.0 в новый буфер t_factor2
    let mut t_factor2 = Mat::default();
    opencv::core::subtract(
        &t_factor1,
        &Scalar::new(1.0, 0.0, 0.0, 0.0),
        &mut t_factor2,
        &Mat::default(),
        -1,
    )
    .map_err(|e| e.to_string())?;

    // Умножаем на коэффициент k в новый буфер t_factor3
    let mut t_factor3 = Mat::default();
    opencv::core::multiply(
        &t_factor2,
        &Scalar::new(k.into(), 0.0, 0.0, 0.0),
        &mut t_factor3,
        1.0,
        -1,
    )
    .map_err(|e| e.to_string())?;

    // Прибавляем 1.0 в финальный t_factor_final
    let mut t_factor_final = Mat::default();
    opencv::core::add(
        &t_factor3,
        &Scalar::new(1.0, 0.0, 0.0, 0.0),
        &mut t_factor_final,
        &Mat::default(),
        -1,
    )
    .map_err(|e| e.to_string())?;

    let mut final_threshold = Mat::default();
    opencv::core::multiply(&mean_f32, &t_factor_final, &mut final_threshold, 1.0, -1)
        .map_err(|e| e.to_string())?;

    let mut dest_threshold_u8 = Mat::default();
    final_threshold
        .convert_to(&mut dest_threshold_u8, opencv::core::CV_8U, 1.0, 0.0)
        .map_err(|e| e.to_string())?;

    let mut binary = Mat::default();
    // opencv::core::compare(&gray, &dest_threshold_u8, &mut binary, opencv::core::CMP_GT)
    // .map_err(|e| e.to_string())?;
    // Патч: меняем CMP_GT на CMP_LT, чтобы буквы остались черными,
    // а серая бумага выгорела в белый #FFFFFF
    // и серый фон открытой крышки Эпсона выгорел в чистый белый #FFFFFF
    opencv::core::compare(&gray, &dest_threshold_u8, &mut binary, opencv::core::CMP_LT)
        .map_err(|e| e.to_string())?;

    Ok(binary)
}
