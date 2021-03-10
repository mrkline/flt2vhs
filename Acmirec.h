/*
** Name: ACMIREC.H
** Description:
**		Recorder class for writing an ACMI recording in raw data format.
**		Types of records are defined here.
** History:
**		13-oct-97 (edg)
**			We go dancing in.....
** Here we are in 21 october 2017, 20 years after ! (loitho)
*/
#ifndef _ACMIREC_H_
#define _ACMIREC_H_

#include <cstdint>

////////////////////////////////////////////////////////////////////////////////
////////////////////////////////////////////////////////////////////////////////
////////////////////////////////////////////////////////////////////////////////
/*
** These are the enumerated record types
*/
enum
{
	ACMIRecGenPosition = 0,
	ACMIRecMissilePosition,
	ACMIRecFeaturePosition,
	ACMIRecAircraftPosition,
	ACMIRecTracerStart,
	ACMIRecStationarySfx,
	ACMIRecMovingSfx,
	ACMIRecSwitch,
	ACMIRecDOF,
	ACMIRecChaffPosition,
	ACMIRecFlarePosition,
	ACMIRecTodOffset,
	ACMIRecFeatureStatus,
	ACMICallsignList,
	ACMIRecMaxTypes
};

////////////////////////////////////////////////////////////////////////////////
////////////////////////////////////////////////////////////////////////////////
////////////////////////////////////////////////////////////////////////////////
/*
** Record structure typedefs for each type of record
*/

//
// ACMIRecHeader
// this struct is common thru all record types as a record header
//
#pragma pack (push, pack1, 1)
typedef struct ACMIRecHeader
{
	uint8_t		type = 0;		// one of the ennumerated types
	float		time = 0.0;		// time stamp
} ACMIRecHeader;
#pragma pack (pop, pack1)

//
// ACMIGenPositionData
// General position data
//
#pragma pack (push, pack1, 1)
typedef struct ACMIGenPositionData
{
	int32_t		type = 0;			// base type for creating simbase object
	int32_t	uniqueID = 0;		// identifier of instance
	float	x = 0.0;
	float	y = 0;
	float	z = 0;
	float	yaw = 0;
	float	pitch = 0;
	float 	roll = 0;
} ACMIGenPositionData;
#pragma pack (pop, pack1)

//
// ACMIFeaturePositionData
// General position data
//
#pragma pack (push, pack1, 1)
typedef struct ACMIFeaturePositionData
{
	int32_t		type = 0;			// base type for creating simbase object
	int32_t	uniqueID = 0;		// identifier of instance
	int32_t	leadUniqueID = 0;	// id of lead component (for bridges. bases etc)
	int32_t		slot = 0;			// slot number in component list
	int32_t		specialFlags = 0;   // campaign feature flag
	float	x = 0;
	float	y = 0;
	float	z = 0;
	float	yaw = 0;
	float	pitch = 0;
	float 	roll = 0;
} ACMIFeaturePositionData;
#pragma pack (pop, pack1)

//
// ACMISwitchData
// General position data
//
#pragma pack (push, pack1, 1)
typedef struct ACMISwitchData
{
	int32_t		type = 0;			// base type for creating simbase object
	int32_t	uniqueID = 0;		// identifier of instance
	int32_t		switchNum = 0;
	int32_t		switchVal = 0;
	int32_t		prevSwitchVal = 0;
} ACMISwitchData;
#pragma pack (pop, pack1)

//
// ACMIFeatureStatusData
// Feature status change data
//
#pragma pack (push, pack1, 1)
typedef struct ACMIFeatureStatusData
{
	int32_t	uniqueID = 0;		// identifier of instance
	int32_t		newStatus = 0;
	int32_t		prevStatus = 0;
} ACMIFeatureStatusData;
#pragma pack (pop, pack1)

//
// ACMIDOFData
// General position data
//
#pragma pack (push, pack1, 1)
typedef struct ACMIDOFData
{
	int32_t		type = 0;			// base type for creating simbase object
	int32_t	uniqueID = 0;		// identifier of instance
	int32_t		DOFNum = 0;
	float	DOFVal = 0.0;
	float	prevDOFVal = 0.0;
} ACMIDOFData;
#pragma pack (pop, pack1)

//
// ACMITracerStartData
// Starting pos and velocity of tracer rounds
//
#pragma pack (push, pack1, 1)
typedef struct ACMITracerStartData
{
	// initial values
	float	x = 0.0;
	float	y = 0.0;
	float	z = 0.0;
	float	dx = 0.0;
	float	dy = 0.0;
	float 	dz = 0.0;
} ACMITracerStartData;
#pragma pack (pop, pack1)

//
// ACMIStationarySfxData
// Starting pos of a staionay special sfx
//
#pragma pack (push, pack1, 1)
typedef struct ACMIStationarySfxData
{
	int32_t		type = 0;		// sfx type
	float	x = 0.0;			// position
	float	y = 0.0;
	float	z = 0.0;
	float	timeToLive = 0.0;
	float	scale = 0.0;
} ACMIStationarySfxData;
#pragma pack (pop, pack1)

//
// ACMIMovingSfxData
// Starting pos of a staionay special sfx
//
#pragma pack (push, pack1, 1)
typedef struct ACMIMovingSfxData
{
	int32_t		type = 0;		// sfx type
	int32_t		user = 0;		// misc data
	int32_t		flags = 0;
	float	x = 0.0;		// position
	float	y = 0.0;
	float	z = 0.0;
	float	dx = 0.0;		// vector
	float	dy = 0.0;
	float	dz = 0.0;
	float	timeToLive = 0.0;
	float	scale = 0.0;
} ACMIMovingSfxData;
#pragma pack (pop, pack1)

// these are the actual I/O records
#pragma pack (push, pack1, 1)

typedef struct ACMIMovingSfxRecord
{
	ACMIRecHeader			hdr;
	ACMIMovingSfxData		data;
} ACMIMovingSfxRecord;

typedef struct ACMIStationarySfxRecord
{
	ACMIRecHeader				hdr;
	ACMIStationarySfxData		data;
} ACMIStationarySfxRecord;

typedef struct ACMIGenPositionRecord
{
	ACMIRecHeader				hdr;
	ACMIGenPositionData			data;
} ACMIGenPositionRecord;

typedef struct ACMIMissilePositionRecord
{
	ACMIRecHeader				hdr;
	ACMIGenPositionData			data;
} ACMIMissilePositionRecord;

typedef struct ACMITodOffsetRecord
{
	ACMIRecHeader				hdr;
} ACMITodOffsetRecord;

typedef struct ACMIChaffPositionRecord
{
	ACMIRecHeader				hdr;
	ACMIGenPositionData			data;
} ACMIChaffPositionRecord;

typedef struct ACMIFlarePositionRecord
{
	ACMIRecHeader				hdr;
	ACMIGenPositionData			data;
} ACMIFlarePositionRecord;

typedef struct ACMIAircraftPositionRecord
{
	ACMIRecHeader				hdr;
	ACMIGenPositionData			data;
	int32_t						RadarTarget;
	
} ACMIAircraftPositionRecord;

typedef struct ACMIFeaturePositionRecord
{
	ACMIRecHeader				hdr;
	ACMIFeaturePositionData		data;
} ACMIFeaturePositionRecord;

typedef struct ACMIFeatureStatusRecord
{
	ACMIRecHeader				hdr;
	ACMIFeatureStatusData		data;
} ACMIFeatureStatusRecord;

typedef struct ACMITracerStartRecord
{
	ACMIRecHeader				hdr;
	ACMITracerStartData			data;
} ACMITracerStartRecord;

typedef struct ACMISwitchRecord
{
	ACMIRecHeader   hdr;
	ACMISwitchData  data;
} ACMISwitchRecord;

typedef struct ACMIDOFRecord
{
	ACMIRecHeader	hdr;
	ACMIDOFData		data;
} ACMIDOFRecord;
#pragma pack (pop, pack1)





////////////////////////////////////////////////////////////////////////////////
////////////////////////////////////////////////////////////////////////////////
////////////////////////////////////////////////////////////////////////////////

#pragma pack (1)
struct ACMI_CallRec
{
	char label[16];
	int32_t teamColor;
};
#pragma pack()

#endif  // _ACMIREC_H_

