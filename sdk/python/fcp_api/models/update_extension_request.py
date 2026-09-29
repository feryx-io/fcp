from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.extension_target_response_type_0 import ExtensionTargetResponseType0
  from ..models.extension_target_response_type_1 import ExtensionTargetResponseType1
  from ..models.extension_target_response_type_2 import ExtensionTargetResponseType2
  from ..models.extension_target_response_type_3 import ExtensionTargetResponseType3





T = TypeVar("T", bound="UpdateExtensionRequest")



@_attrs_define
class UpdateExtensionRequest:
    """ Update extension request.

        Attributes:
            target (ExtensionTargetResponseType0 | ExtensionTargetResponseType1 | ExtensionTargetResponseType2 |
                ExtensionTargetResponseType3 | None | Unset):
            description (None | str | Unset):
            location_id (None | str | Unset):
            enabled (bool | None | Unset):
     """

    target: ExtensionTargetResponseType0 | ExtensionTargetResponseType1 | ExtensionTargetResponseType2 | ExtensionTargetResponseType3 | None | Unset = UNSET
    description: None | str | Unset = UNSET
    location_id: None | str | Unset = UNSET
    enabled: bool | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.extension_target_response_type_0 import ExtensionTargetResponseType0 # noqa: PLC0415
        from ..models.extension_target_response_type_1 import ExtensionTargetResponseType1 # noqa: PLC0415
        from ..models.extension_target_response_type_2 import ExtensionTargetResponseType2 # noqa: PLC0415
        from ..models.extension_target_response_type_3 import ExtensionTargetResponseType3 # noqa: PLC0415
        target: dict[str, Any] | None | Unset
        if isinstance(self.target, Unset):
            target = UNSET
        elif isinstance(self.target, ExtensionTargetResponseType0):
            target = self.target.to_dict()
        elif isinstance(self.target, ExtensionTargetResponseType1):
            target = self.target.to_dict()
        elif isinstance(self.target, ExtensionTargetResponseType2):
            target = self.target.to_dict()
        elif isinstance(self.target, ExtensionTargetResponseType3):
            target = self.target.to_dict()
        else:
            target = self.target

        description: None | str | Unset
        if isinstance(self.description, Unset):
            description = UNSET
        else:
            description = self.description

        location_id: None | str | Unset
        if isinstance(self.location_id, Unset):
            location_id = UNSET
        else:
            location_id = self.location_id

        enabled: bool | None | Unset
        if isinstance(self.enabled, Unset):
            enabled = UNSET
        else:
            enabled = self.enabled


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if target is not UNSET:
            field_dict["target"] = target
        if description is not UNSET:
            field_dict["description"] = description
        if location_id is not UNSET:
            field_dict["location_id"] = location_id
        if enabled is not UNSET:
            field_dict["enabled"] = enabled

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.extension_target_response_type_0 import ExtensionTargetResponseType0 # noqa: PLC0415
        from ..models.extension_target_response_type_1 import ExtensionTargetResponseType1 # noqa: PLC0415
        from ..models.extension_target_response_type_2 import ExtensionTargetResponseType2 # noqa: PLC0415
        from ..models.extension_target_response_type_3 import ExtensionTargetResponseType3 # noqa: PLC0415
        d = dict(src_dict)
        def _parse_target(data: object) -> ExtensionTargetResponseType0 | ExtensionTargetResponseType1 | ExtensionTargetResponseType2 | ExtensionTargetResponseType3 | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                componentsschemas_extension_target_response_type_0 = ExtensionTargetResponseType0.from_dict(data)



                return componentsschemas_extension_target_response_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                componentsschemas_extension_target_response_type_1 = ExtensionTargetResponseType1.from_dict(data)



                return componentsschemas_extension_target_response_type_1
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                componentsschemas_extension_target_response_type_2 = ExtensionTargetResponseType2.from_dict(data)



                return componentsschemas_extension_target_response_type_2
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                componentsschemas_extension_target_response_type_3 = ExtensionTargetResponseType3.from_dict(data)



                return componentsschemas_extension_target_response_type_3
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(ExtensionTargetResponseType0 | ExtensionTargetResponseType1 | ExtensionTargetResponseType2 | ExtensionTargetResponseType3 | None | Unset, data)

        target = _parse_target(d.pop("target", UNSET))


        def _parse_description(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        description = _parse_description(d.pop("description", UNSET))


        def _parse_location_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        location_id = _parse_location_id(d.pop("location_id", UNSET))


        def _parse_enabled(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        enabled = _parse_enabled(d.pop("enabled", UNSET))


        update_extension_request = cls(
            target=target,
            description=description,
            location_id=location_id,
            enabled=enabled,
        )


        update_extension_request.additional_properties = d
        return update_extension_request

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
