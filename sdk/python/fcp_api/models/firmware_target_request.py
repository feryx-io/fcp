from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.rollout_stage import RolloutStage
from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="FirmwareTargetRequest")



@_attrs_define
class FirmwareTargetRequest:
    """ A target, made or changed.

        Attributes:
            vendor (str):
            image_id (str):
            model_pattern (None | str | Unset): Unset: every model of the vendor.
            stage (None | RolloutStage | Unset):
            pilot_devices (list[str] | Unset):
     """

    vendor: str
    image_id: str
    model_pattern: None | str | Unset = UNSET
    stage: None | RolloutStage | Unset = UNSET
    pilot_devices: list[str] | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        vendor = self.vendor

        image_id = self.image_id

        model_pattern: None | str | Unset
        if isinstance(self.model_pattern, Unset):
            model_pattern = UNSET
        else:
            model_pattern = self.model_pattern

        stage: None | str | Unset
        if isinstance(self.stage, Unset):
            stage = UNSET
        elif isinstance(self.stage, RolloutStage):
            stage = self.stage.value
        else:
            stage = self.stage

        pilot_devices: list[str] | Unset = UNSET
        if not isinstance(self.pilot_devices, Unset):
            pilot_devices = self.pilot_devices




        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "vendor": vendor,
            "image_id": image_id,
        })
        if model_pattern is not UNSET:
            field_dict["model_pattern"] = model_pattern
        if stage is not UNSET:
            field_dict["stage"] = stage
        if pilot_devices is not UNSET:
            field_dict["pilot_devices"] = pilot_devices

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        vendor = d.pop("vendor")

        image_id = d.pop("image_id")

        def _parse_model_pattern(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        model_pattern = _parse_model_pattern(d.pop("model_pattern", UNSET))


        def _parse_stage(data: object) -> None | RolloutStage | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                stage_type_0 = RolloutStage(data)



                return stage_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(None | RolloutStage | Unset, data)

        stage = _parse_stage(d.pop("stage", UNSET))


        pilot_devices = cast(list[str], d.pop("pilot_devices", UNSET))


        firmware_target_request = cls(
            vendor=vendor,
            image_id=image_id,
            model_pattern=model_pattern,
            stage=stage,
            pilot_devices=pilot_devices,
        )


        firmware_target_request.additional_properties = d
        return firmware_target_request

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
