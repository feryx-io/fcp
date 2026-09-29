from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime

if TYPE_CHECKING:
  from ..models.validation_response import ValidationResponse





T = TypeVar("T", bound="DeviceValidationResponse")



@_attrs_define
class DeviceValidationResponse:
    """ 
        Attributes:
            validation (ValidationResponse):
            device_id (str):
            name (str):
            validated (bool):
            validated_at (datetime.datetime | None | Unset):
     """

    validation: ValidationResponse
    device_id: str
    name: str
    validated: bool
    validated_at: datetime.datetime | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.validation_response import ValidationResponse # noqa: PLC0415
        validation = self.validation.to_dict()

        device_id = self.device_id

        name = self.name

        validated = self.validated

        validated_at: None | str | Unset
        if isinstance(self.validated_at, Unset):
            validated_at = UNSET
        elif isinstance(self.validated_at, datetime.datetime):
            validated_at = self.validated_at.isoformat()
        else:
            validated_at = self.validated_at


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "validation": validation,
            "device_id": device_id,
            "name": name,
            "validated": validated,
        })
        if validated_at is not UNSET:
            field_dict["validated_at"] = validated_at

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.validation_response import ValidationResponse # noqa: PLC0415
        d = dict(src_dict)
        validation = ValidationResponse.from_dict(d.pop("validation"))




        device_id = d.pop("device_id")

        name = d.pop("name")

        validated = d.pop("validated")

        def _parse_validated_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                validated_at_type_0 = datetime.datetime.fromisoformat(data)



                return validated_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        validated_at = _parse_validated_at(d.pop("validated_at", UNSET))


        device_validation_response = cls(
            validation=validation,
            device_id=device_id,
            name=name,
            validated=validated,
            validated_at=validated_at,
        )


        device_validation_response.additional_properties = d
        return device_validation_response

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
